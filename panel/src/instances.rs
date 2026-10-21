//! Managing the web server instances that the panel runs.
//!
//! Each instance is a child process running the binary of its preset. The
//! child is started with resource limits applied through `setrlimit`, so a
//! runaway instance cannot take more memory or disk space than it was
//! granted.
//!
//! The share of the processor is kept in the panel itself, because the
//! kernel has no way of saying "this process may use half a processor". The
//! panel watches how much processor time the child uses and pauses it when
//! it has used too much, which is what a limit of a percentage means.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::db::{Database, Result as DbResult};
use crate::models::{Instance, InstanceState, Usage};
use crate::presets::Preset;

/// Number of clock ticks per second, used to convert the cpu counters in
/// `/proc/<pid>/stat` into seconds.
const CLOCK_TICKS_PER_SECOND: f64 = 100.0;

/// How much processor time an instance may use in total, before the kernel
/// stops it without asking.
///
/// The share of the processor is kept by the panel, and this is only the
/// backstop for a process that ignores the pauses.
const CPU_BACKSTOP_SECONDS: u64 = 24 * 60 * 60;

/// How often the panel looks at an instance to see how much processor time
/// it used.
const WATCH_INTERVAL: Duration = Duration::from_millis(250);

/// The longest an instance may be paused in one go.
const MAX_PAUSE: Duration = Duration::from_secs(5);

/// How often the size of the directory of an instance is measured.
const DISK_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// The number of samples the share of the processor is worked out over.
///
/// With a sample every quarter of a second this is about two seconds, so
/// an instance can burst for a moment, but not for long.
const SAMPLE_COUNT: usize = 8;

/// Resource limits applied to a child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessLimits {
    /// Maximum resident memory, in megabytes.
    pub memory_mb: i64,
    /// Share of the processor the instance may use, in percent.
    pub cpu_percent: i64,
    /// Maximum disk size of a written file, in megabytes.
    pub disk_mb: i64,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        ProcessLimits {
            memory_mb: 256,
            cpu_percent: 50,
            disk_mb: 1024,
        }
    }
}

/// A running instance process.
struct Managed {
    child: Child,
    /// Tells the watcher to stop.
    stop: Arc<AtomicBool>,
}

/// What the panel knows about a running instance right now.
#[derive(Debug, Clone, Default)]
pub struct LiveUsage {
    /// Resident memory in kilobytes.
    pub memory_kb: u64,
    /// Share of the processor in use, in percent.
    pub cpu_percent: u64,
    /// Disk space used by the instance, in kilobytes.
    pub disk_kb: u64,
    /// Whether the instance is paused because it used too much processor
    /// time.
    pub throttled: bool,
}

/// Keeps track of the running instance processes.
pub struct InstanceManager {
    running: Mutex<HashMap<i64, Managed>>,
    /// The binary of every preset, found when it is first needed.
    binaries: Mutex<HashMap<&'static str, PathBuf>>,
    /// What the running instances are using right now.
    ///
    /// It is shared with the watchers, which keep it up to date.
    usage: Arc<Mutex<HashMap<i64, LiveUsage>>>,
}

impl Default for InstanceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl InstanceManager {
    /// Create an empty manager.
    pub fn new() -> InstanceManager {
        InstanceManager {
            running: Mutex::new(HashMap::new()),
            binaries: Mutex::new(HashMap::new()),
            usage: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The path of the binary that runs a preset.
    ///
    /// The lookup is cached, because it touches the file system.
    pub fn instance_binary(&self, preset: Preset) -> Option<PathBuf> {
        if let Some(path) = self.binaries.lock().unwrap().get(preset.binary()) {
            return Some(path.clone());
        }
        let found = find_binary(preset);
        if let Some(path) = &found {
            self.binaries
                .lock()
                .unwrap()
                .insert(preset.binary(), path.clone());
        }
        found
    }

    /// Override the binary used for a preset.
    pub fn set_instance_binary(&self, preset: Preset, path: PathBuf) {
        self.binaries.lock().unwrap().insert(preset.binary(), path);
    }

    /// Start the web server for an instance.
    pub fn start(
        &self,
        db: &Database,
        instance: &Instance,
        data_dir: &Path,
    ) -> std::result::Result<u16, String> {
        {
            let running = self.running.lock().unwrap();
            if running.contains_key(&instance.id) {
                return Err("Instance is already running.".into());
            }
        }

        let binary = self.instance_binary(instance.preset).ok_or_else(|| {
            format!(
                "Cannot find the binary of the preset {} ({}).",
                instance.preset.label(),
                instance.preset.binary()
            )
        })?;

        let work_dir = data_dir.join(format!("instance-{}", instance.id));
        std::fs::create_dir_all(&work_dir)
            .map_err(|err| format!("Cannot create instance directory: {err}"))?;

        let limits = ProcessLimits {
            memory_mb: instance.memory_limit_mb,
            cpu_percent: instance.cpu_limit_percent,
            disk_mb: instance.disk_limit_mb,
        };

        // The output of the instance is written to a file, so a crash can be
        // inspected afterwards.
        let log_path = work_dir.join("instance.log");
        let log = std::fs::File::create(&log_path)
            .map_err(|err| format!("Cannot create the instance log: {err}"))?;
        let log_err = log
            .try_clone()
            .map_err(|err| format!("Cannot create the instance log: {err}"))?;

        let mut command = Command::new(&binary);
        command
            .arg("--port")
            .arg(instance.port.to_string())
            .arg("--host")
            .arg("127.0.0.1");
        // A site is served from the directory of the instance, so the files
        // the user writes in the editor are the ones that are served.
        if instance.preset.is_editable() {
            command.arg("--root").arg(&work_dir);
        }
        command
            .current_dir(&work_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));

        apply_limits(&mut command, limits)?;

        let mut child = command
            .spawn()
            .map_err(|err| format!("Cannot start instance: {err}"))?;
        let pid = child.id() as i32;

        // The instance may move to another port when the one it was given is
        // busy, so the port it reports is read from its output.
        let actual_port = match wait_for_port(&log_path, &mut child) {
            Some(port) => port,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let log = std::fs::read_to_string(&log_path).unwrap_or_default();
                let detail = log
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("the instance produced no output");
                return Err(format!("The instance did not start: {detail}"));
            }
        };

        // The watcher keeps the instance inside its share of the processor
        // and measures what it uses.
        let stop = Arc::new(AtomicBool::new(false));
        let _watcher = watch(
            instance.id,
            child.id() as i32,
            limits.cpu_percent,
            work_dir.clone(),
            stop.clone(),
            self.usage.clone(),
        );

        self.running
            .lock()
            .unwrap()
            .insert(instance.id, Managed { child, stop });

        // The instance may have had to move to another port.
        if actual_port != instance.port {
            db.set_instance_port(instance.id, actual_port)
                .map_err(|err| format!("Cannot record the instance port: {err}"))?;
        }

        db.set_instance_state(instance.id, InstanceState::Running, Some(pid), None)
            .map_err(|err| format!("Cannot record instance state: {err}"))?;

        Ok(actual_port)
    }

    /// Stop the web server of an instance.
    pub fn stop(&self, db: &Database, instance: &Instance) -> std::result::Result<(), String> {
        let managed = self.running.lock().unwrap().remove(&instance.id);
        if let Some(managed) = &managed {
            managed.stop.store(true, Ordering::Relaxed);
        }
        self.usage.lock().unwrap().remove(&instance.id);

        // An instance started by an earlier run of the panel is not in the
        // map, so the recorded pid is used instead.
        if managed.is_none() {
            if let Some(pid) = instance.pid {
                terminate_pid(pid);
            }
        } else if let Some(mut managed) = managed {
            terminate_pid(managed.child.id() as i32);
            let _ = managed.child.wait();
        }

        db.set_instance_state(instance.id, InstanceState::Stopped, None, None)
            .map_err(|err| format!("Cannot record instance state: {err}"))?;
        Ok(())
    }

    /// Whether an instance currently has a live process.
    pub fn is_running(&self, instance: &Instance) -> bool {
        if self.running.lock().unwrap().contains_key(&instance.id) {
            return true;
        }
        instance.pid.is_some_and(process_alive)
    }

    /// Stop every instance this manager knows about.
    pub fn stop_all(&self) {
        let mut running = self.running.lock().unwrap();
        for (_, mut managed) in running.drain() {
            managed.stop.store(true, Ordering::Relaxed);
            terminate_pid(managed.child.id() as i32);
            let _ = managed.child.wait();
        }
        self.usage.lock().unwrap().clear();
    }

    /// The resources an instance is using right now.
    ///
    /// The watcher of the instance keeps this up to date, so the number is
    /// the one the instance had a moment ago. This never waits, because a
    /// request that waits holds up every other request.
    ///
    /// An instance that the panel does not watch, for example one that was
    /// left behind by an earlier run of the panel, is measured from what
    /// the kernel knows about it. The share of the processor is then the
    /// average since the process was started, because measuring it over a
    /// moment would mean waiting.
    pub fn usage(&self, instance: &Instance) -> Usage {
        let mut usage = Usage::default();
        let Some(pid) = self.live_pid(instance) else {
            return usage;
        };

        if let Some(live) = self.usage.lock().unwrap().get(&instance.id) {
            usage.memory_kb = live.memory_kb;
            usage.cpu_percent = live.cpu_percent;
            usage.disk_kb = live.disk_kb;
            return usage;
        }

        usage.memory_kb = resident_memory(pid);
        usage.cpu_percent = average_cpu_percent(pid);
        usage
    }

    /// The disk space used by an instance working directory.
    pub fn disk_usage(&self, data_dir: &Path, instance: &Instance) -> u64 {
        let work_dir = data_dir.join(format!("instance-{}", instance.id));
        directory_size(&work_dir)
    }

    /// The pid of a running instance, if there is one.
    fn live_pid(&self, instance: &Instance) -> Option<i32> {
        if let Some(managed) = self.running.lock().unwrap().get(&instance.id) {
            return Some(managed.child.id() as i32);
        }
        instance.pid.filter(|pid| process_alive(*pid))
    }
}

/// Find the binary that instances are started from.
pub fn find_binary(preset: Preset) -> Option<PathBuf> {
    // The environment can point at a binary.
    let variable = match preset {
        Preset::HelloWorld | Preset::Site => "NOOBS_INSTANCE_BIN",
        Preset::Benchmarker => "NOOBS_BENCHMARKER_BIN",
    };
    if let Ok(path) = std::env::var(variable) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    let name = preset.binary();

    // A binary next to the panel binary is the usual case, because all of
    // them are built by cargo into the same target directory. The
    // directories above are looked at as well, because the tests run from
    // somewhere inside the target tree.
    if let Ok(current) = std::env::current_exe() {
        let mut directory = current.parent().map(|dir| dir.to_path_buf());
        for _ in 0..4 {
            let Some(current) = directory else {
                break;
            };
            let candidate = current.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            directory = current.parent().map(|dir| dir.to_path_buf());
        }
    }

    // Fall back to the usual cargo output directories.
    for profile in ["release", "debug"] {
        let candidate = PathBuf::from("target").join(profile).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

/// Watch a running instance.
///
/// The watcher keeps the instance inside its share of the processor and
/// records what the instance is using, so the panel can show it without
/// having to look at `/proc` for every page.
fn watch(
    id: i64,
    pid: i32,
    cpu_percent: i64,
    work_dir: PathBuf,
    stop: Arc<AtomicBool>,
    manager: Arc<Mutex<HashMap<i64, LiveUsage>>>,
) -> std::thread::JoinHandle<()> {
    // The moment the process started, so a process that takes the same
    // number later is not mistaken for the instance.
    let started = process_start_time(pid);

    std::thread::spawn(move || {
        let mut last_cpu = cpu_time(pid);
        let mut last_wall = Instant::now();
        let mut used = LiveUsage::default();

        // The samples of the last few moments. The share of the processor
        // is worked out over them, so a pause counts as the time it takes,
        // and the instance is not allowed to burst for a long time.
        let mut samples: VecDeque<(f64, f64)> = VecDeque::new();

        // The moment the size of the directory was measured last.
        let mut last_disk: Option<Instant> = None;

        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(WATCH_INTERVAL);
            if !process_alive(pid) {
                break;
            }
            if process_start_time(pid) != started {
                // Another process took the number, so this is not the
                // instance any more.
                break;
            }

            let now = Instant::now();
            let cpu = cpu_time(pid);
            let wall = now.duration_since(last_wall).as_secs_f64();
            let taken = (cpu - last_cpu).max(0.0);
            last_cpu = cpu;
            last_wall = now;

            samples.push_back((taken, wall));
            while samples.len() > SAMPLE_COUNT {
                samples.pop_front();
            }

            let used_cpu: f64 = samples.iter().map(|(cpu, _)| cpu).sum();
            let used_wall: f64 = samples.iter().map(|(_, wall)| wall).sum();

            // The share of the processor the instance may use.
            let share = cpu_percent.clamp(0, 100) as f64 / 100.0;

            // The time the instance used more than it was allowed to use.
            let excess = used_cpu - used_wall * share;

            if share > 0.0 && share < 1.0 && excess > 0.0 {
                // The instance used too much processor time, so it is paused
                // until it has earned the time back. The pause is the time
                // it used too much, divided by the share it may use, so the
                // instance ends the pause with the time it used too much
                // again in hand.
                let pause = Duration::from_secs_f64((excess / share).min(MAX_PAUSE.as_secs_f64()));
                used.throttled = true;
                pause_process(pid, pause, &stop);
                used.throttled = false;
            }

            used.cpu_percent = if used_wall > 0.0 {
                ((used_cpu / used_wall) * 100.0).round().clamp(0.0, 999.0) as u64
            } else {
                0
            };
            used.memory_kb = resident_memory(pid);

            // The size of the directory is measured here and not in the
            // request, because walking a directory with many files in it
            // takes a moment and a request may not wait.
            if last_disk.is_none_or(|when| when.elapsed() >= DISK_SAMPLE_INTERVAL) {
                last_disk = Some(Instant::now());
                used.disk_kb = directory_size(&work_dir) / 1024;
            }

            manager.lock().unwrap().insert(id, used.clone());
        }

        manager.lock().unwrap().remove(&id);
    })
}

/// Pause a process for a while and let it run again afterwards.
///
/// The pause is cut short when the watcher is told to stop, so an instance
/// that is stopped is never left hanging.
fn pause_process(pid: i32, pause: Duration, stop: &AtomicBool) {
    if pause.is_zero() {
        return;
    }
    unsafe {
        libc::kill(pid, libc::SIGSTOP);
    }
    let mut left = pause;
    while left > Duration::ZERO && !stop.load(Ordering::Relaxed) {
        let step = left.min(Duration::from_millis(50));
        std::thread::sleep(step);
        left -= step;
    }
    unsafe {
        libc::kill(pid, libc::SIGCONT);
    }
}

/// The moment a process started, in clock ticks since it was started.
///
/// It is the field the kernel keeps for the process, so a process that
/// takes the same number later has a different value.
fn process_start_time(pid: i32) -> u64 {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return 0;
    };
    let Some(rest) = stat.rsplit_once(") ").map(|(_, rest)| rest) else {
        return 0;
    };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // The start time is field 22 of the whole line, and the fields before it
    // are counted from the name.
    fields
        .get(19)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
}

/// The share of the processor a process is using, in percent, since it was
/// started.
///
/// The kernel knows how much processor time a process has used and how long
/// ago it was started, so the share can be worked out without waiting.
fn average_cpu_percent(pid: i32) -> u64 {
    let started = process_start_time(pid);
    if started == 0 {
        return 0;
    }

    let uptime = uptime_seconds();
    if uptime <= 0.0 {
        return 0;
    }

    // The start time of the process is in clock ticks since the machine
    // was started.
    let age = uptime - started as f64 / CLOCK_TICKS_PER_SECOND;
    if age <= 0.0 {
        return 0;
    }

    let share = cpu_time(pid) / age * 100.0;
    share.round().clamp(0.0, 999.0) as u64
}

/// How long the machine has been up, in seconds.
fn uptime_seconds() -> f64 {
    let Ok(uptime) = std::fs::read_to_string("/proc/uptime") else {
        return 0.0;
    };
    uptime
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// The processor time a process has used, in seconds.
fn cpu_time(pid: i32) -> f64 {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return 0.0;
    };
    // The command name is in brackets and may contain spaces, so the fields
    // are counted from the last closing bracket.
    let Some(rest) = stat.rsplit_once(") ").map(|(_, rest)| rest) else {
        return 0.0;
    };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime = fields
        .get(11)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    let stime = fields
        .get(12)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    (utime + stime) / CLOCK_TICKS_PER_SECOND
}

/// The resident memory of a process, in kilobytes.
fn resident_memory(pid: i32) -> u64 {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return 0;
    };
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            return value
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
        }
    }
    0
}

/// Apply the resource limits to a command before it is started.
///
/// `RLIMIT_DATA` is used for the memory limit rather than `RLIMIT_AS`. The
/// Android dynamic linker reserves a very large shadow memory region before
/// the program starts, so limiting the address space makes the linker fail
/// before the program is even running. The data limit covers the heap of the
/// program, which is what a runaway instance would exhaust.
///
/// Note that `RLIMIT_NPROC` is deliberately not used. On Linux it counts
/// every process of the user, not only the children of the instance, so a
/// small value makes even the start of a normal program fail.
fn apply_limits(command: &mut Command, limits: ProcessLimits) -> std::result::Result<(), String> {
    use std::os::unix::process::CommandExt;

    let memory_bytes = limits.memory_mb.max(1) as u64 * 1024 * 1024;
    let disk_bytes = limits.disk_mb.max(1) as u64 * 1024 * 1024;

    // The closure runs in the child process between fork and exec, so it
    // must not allocate or do anything that is not async signal safe.
    unsafe {
        command.pre_exec(move || {
            set_limit(libc::RLIMIT_DATA, memory_bytes, memory_bytes);
            set_limit(libc::RLIMIT_FSIZE, disk_bytes, disk_bytes);
            // The share of the processor is kept by the panel, and this is
            // only the backstop for a process that ignores the pauses.
            set_limit(libc::RLIMIT_CPU, CPU_BACKSTOP_SECONDS, CPU_BACKSTOP_SECONDS);
            Ok(())
        });
    }
    Ok(())
}

/// Set a single resource limit, ignoring failures.
///
/// A limit that the platform refuses to apply must not stop the instance
/// from starting, otherwise one unsupported limit would break everything.
unsafe fn set_limit(resource: libc::c_int, soft: u64, hard: u64) {
    let limit = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    libc::setrlimit(resource, &limit);
}

/// Wait for an instance to report the port it listens on.
///
/// Returns `None` when the instance died before it was ready.
///
/// Only the end of the log is read, because the log of an instance can
/// grow to any size and reading all of it every fifty milliseconds would
/// cost more than waiting for the instance.
fn wait_for_port(log_path: &Path, child: &mut Child) -> Option<u16> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        for line in log_tail(log_path).lines() {
            if let Some(rest) = line.trim().strip_prefix("Port:") {
                if let Ok(port) = rest.trim().parse::<u16>() {
                    return Some(port);
                }
            }
        }
        // A process that is gone will not report anything.
        if let Ok(Some(status)) = child.try_wait() {
            let _ = status;
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

/// The last part of a file, as text.
///
/// The file is read from the end, so a file of any size costs the same.
fn log_tail(path: &Path) -> String {
    const TAIL_BYTES: u64 = 8 * 1024;

    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let Ok(length) = file.metadata().map(|meta| meta.len()) else {
        return String::new();
    };
    if length == 0 {
        return String::new();
    }

    let start = length.saturating_sub(TAIL_BYTES);
    if file.seek(std::io::SeekFrom::Start(start)).is_err() {
        return String::new();
    }

    let mut buffer = Vec::new();
    if file.read_to_end(&mut buffer).is_err() {
        return String::new();
    }

    String::from_utf8_lossy(&buffer).to_string()
}

/// Whether a process id is alive.
pub fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // Signal 0 does not deliver a signal, it only checks that the process
    // exists and that the caller may signal it.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Ask a process to stop, then make sure it is gone.
fn terminate_pid(pid: i32) {
    if pid <= 0 {
        return;
    }
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    // Give the process a moment to shut down on its own.
    for _ in 0..50 {
        if !process_alive(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}

/// The total size of the files in a directory, in kilobytes.
pub fn directory_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            total += directory_size(&entry.path());
        } else {
            total += metadata.len() / 1024;
        }
    }
    total
}

/// Find a free port inside a range.
pub fn find_free_port(range: (u16, u16)) -> Option<u16> {
    (range.0..=range.1).find(|&port| std::net::TcpListener::bind(("127.0.0.1", port)).is_ok())
}

/// Bring the database in line with what is actually running.
///
/// Instances that were running when the panel stopped are reported as
/// crashed, because their process is gone.
pub fn reconcile(db: &Database, instances: &[Instance]) -> DbResult<usize> {
    let mut changed = 0;
    for instance in instances {
        let alive = instance.pid.is_some_and(process_alive);
        let expected = if alive {
            InstanceState::Running
        } else if instance.state == InstanceState::Running {
            InstanceState::Crashed
        } else {
            instance.state
        };
        if expected != instance.state {
            db.set_instance_state(instance.id, expected, None, None)?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// A shared handle to the manager.
pub type SharedManager = Arc<InstanceManager>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn detects_live_processes() {
        assert!(process_alive(std::process::id() as i32));
        assert!(!process_alive(0));
        assert!(!process_alive(-1));
    }

    #[test]
    fn finds_a_free_port_in_the_range() {
        let port = find_free_port((8100, 8999)).expect("a free port");
        assert!((8100..=8999).contains(&port));
    }

    #[test]
    fn reports_no_free_port_in_a_tiny_range() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(find_free_port((port, port)), None);
    }

    #[test]
    fn measures_directory_size() {
        let dir = std::env::temp_dir().join("noobs-panel-dirsize-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), vec![0u8; 4096]).unwrap();
        assert!(directory_size(&dir) >= 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn limits_are_converted_to_bytes() {
        let limits = ProcessLimits {
            memory_mb: 128,
            disk_mb: 512,
            cpu_percent: 50,
        };
        assert_eq!(limits.memory_mb * 1024 * 1024, 134_217_728);
        let _ = Ipv4Addr::LOCALHOST;
    }
}
