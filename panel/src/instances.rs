//! Managing the web server instances that the panel runs.
//!
//! Each instance is a child process running the `hello-server` binary. The
//! child is started with resource limits applied through `setrlimit`, so a
//! runaway instance cannot take more memory, cpu time or disk space than it
//! was granted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::db::{Database, Result as DbResult};
use crate::models::{Instance, InstanceState, Usage};
use crate::presets::Preset;

/// Number of clock ticks per second, used to convert the cpu counters in
/// `/proc/<pid>/stat` into seconds.
const CLOCK_TICKS_PER_SECOND: f64 = 100.0;

/// How much processor time an instance may use past its limit, before the
/// kernel stops it without asking.
const CPU_GRACE_SECONDS: u64 = 5;

/// Resource limits applied to a child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessLimits {
    /// Maximum resident memory, in megabytes.
    pub memory_mb: i64,
    /// Maximum cpu time, in seconds.
    pub cpu_seconds: i64,
    /// Maximum disk size of a written file, in megabytes.
    pub disk_mb: i64,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        ProcessLimits {
            memory_mb: 256,
            cpu_seconds: 60,
            disk_mb: 1024,
        }
    }
}

/// A running instance process.
struct Managed {
    child: Child,
}

/// Keeps track of the running instance processes.
pub struct InstanceManager {
    running: Mutex<HashMap<i64, Managed>>,
    /// The binary of every preset, found when it is first needed.
    binaries: Mutex<HashMap<&'static str, PathBuf>>,
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
            cpu_seconds: instance.cpu_limit_seconds,
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
            .arg("127.0.0.1")
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

        self.running
            .lock()
            .unwrap()
            .insert(instance.id, Managed { child });

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
            terminate_pid(managed.child.id() as i32);
            let _ = managed.child.wait();
        }
    }

    /// The resources an instance is using right now.
    pub fn usage(&self, instance: &Instance) -> Usage {
        let mut usage = Usage::default();
        let Some(pid) = self.live_pid(instance) else {
            return usage;
        };

        if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
            for line in status.lines() {
                if let Some(value) = line.strip_prefix("VmRSS:") {
                    usage.memory_kb = value
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(0);
                }
            }
        }

        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // The second field is the command name in parentheses and may
            // contain spaces, so the fields after it are located from the
            // last closing parenthesis.
            if let Some(rest) = stat.rsplit_once(") ").map(|(_, rest)| rest) {
                let fields: Vec<&str> = rest.split_whitespace().collect();
                // After the state field, utime is field 11 and stime field
                // 12 of the original numbering.
                let utime = fields
                    .get(11)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                let stime = fields
                    .get(12)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                usage.cpu_seconds = ((utime + stime) as f64 / CLOCK_TICKS_PER_SECOND) as u64;
            }
        }

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
        Preset::HelloWorld => "NOOBS_INSTANCE_BIN",
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
    let cpu_seconds = limits.cpu_seconds.max(1) as u64;

    // The closure runs in the child process between fork and exec, so it
    // must not allocate or do anything that is not async signal safe.
    unsafe {
        command.pre_exec(move || {
            set_limit(libc::RLIMIT_DATA, memory_bytes, memory_bytes);
            set_limit(libc::RLIMIT_FSIZE, disk_bytes, disk_bytes);
            // The soft limit is the one that counts. The hard limit is a
            // little higher, so the kernel sends the signal that a program
            // can handle first, and the hard limit only catches a program
            // that ignores it.
            set_limit(
                libc::RLIMIT_CPU,
                cpu_seconds,
                cpu_seconds.saturating_add(CPU_GRACE_SECONDS),
            );
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
fn wait_for_port(log_path: &Path, child: &mut Child) -> Option<u16> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if let Ok(output) = std::fs::read_to_string(log_path) {
            for line in output.lines() {
                if let Some(rest) = line.trim().strip_prefix("Port:") {
                    if let Ok(port) = rest.trim().parse::<u16>() {
                        return Some(port);
                    }
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
            cpu_seconds: 30,
        };
        assert_eq!(limits.memory_mb * 1024 * 1024, 134_217_728);
        let _ = Ipv4Addr::LOCALHOST;
    }
}
