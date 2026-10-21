//! Tests for the benchmarker.

use std::io::{BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// The limits the server under test is given.
///
/// They are the same kind of limits the panel gives an instance, so the
/// tests cannot use up the whole machine.
const MEMORY_MB: u64 = 128;
const CPU_SECONDS: u64 = 30;
const DISK_MB: u64 = 64;

/// A running benchmarker.
struct Server {
    child: Child,
    port: u16,
    /// Keeps the output pipe open.
    ///
    /// If the read end of the pipe were closed, the next write by the
    /// server would raise SIGPIPE and kill it.
    _stdout: std::thread::JoinHandle<String>,
}

impl Server {
    /// Start the binary and wait until it reports the port it listens on.
    ///
    /// The port is held busy before the server starts, so the fallback to
    /// the next free port is exercised as well.
    fn start(port: u16) -> Server {
        let _blocker =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).unwrap();

        let mut command = Command::new(env!("CARGO_BIN_EXE_benchmarker"));
        command
            .arg("--port")
            .arg(port.to_string())
            .arg("--host")
            .arg("127.0.0.1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: the limits are set before the process does anything else,
        // and nothing is allocated in the closure.
        unsafe {
            command.pre_exec(|| {
                set_limit(libc::RLIMIT_DATA, MEMORY_MB * 1024 * 1024);
                set_limit(libc::RLIMIT_FSIZE, DISK_MB * 1024 * 1024);
                set_limit(libc::RLIMIT_CPU, CPU_SECONDS);
                Ok(())
            });
        }
        let mut child = command.spawn().expect("failed to start the benchmarker");

        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut port = None;
        for _ in 0..200 {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            if let Some(rest) = line.trim().strip_prefix("Port:") {
                port = rest.trim().parse::<u16>().ok();
                break;
            }
        }

        // Keep draining the output in the background, so the pipe stays open
        // and the server never blocks on a full pipe buffer.
        let pump = std::thread::spawn(move || {
            let mut collected = String::new();
            let _ = reader.read_to_string(&mut collected);
            collected
        });

        Server {
            child,
            port: port.expect("the benchmarker did not report a port"),
            _stdout: pump,
        }
    }

    /// Ask the server for a path.
    fn get(&self, path: &str) -> String {
        let mut stream =
            TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port))
                .expect("the benchmarker should be listening");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Set a resource limit for the process that is started.
fn set_limit(resource: libc::c_int, value: u64) {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: the limit is set before the process does anything else.
    unsafe {
        libc::setrlimit(resource, &limit);
    }
}

#[test]
fn it_serves_a_page_with_the_limits_of_the_process() {
    let server = Server::start(18195);

    let answer = server.get("/");
    assert!(answer.contains("200 OK"), "unexpected answer: {answer}");
    assert!(answer.contains("Benchmarker"));
    // The limits the kernel gave this process are shown.
    assert!(answer.contains("Max data size"));
    assert!(answer.contains("Max cpu time"));

    let answer = server.get("/health");
    assert!(answer.contains("ok"));
}

#[test]
fn the_memory_limit_stops_the_allocation() {
    let server = Server::start(18196);

    let answer = server.get("/stress/memory");
    assert!(answer.contains("200 OK"), "unexpected answer: {answer}");
    assert!(
        answer.contains("memory limit stopped this process"),
        "unexpected answer: {answer}"
    );
    // The server is still alive after the test.
    let answer = server.get("/health");
    assert!(answer.contains("ok"));
}

#[test]
fn the_disk_limit_stops_the_writing() {
    let server = Server::start(18197);

    let answer = server.get("/stress/disk");
    assert!(answer.contains("200 OK"), "unexpected answer: {answer}");
    assert!(
        answer.contains("disk limit stopped this process"),
        "unexpected answer: {answer}"
    );
}
