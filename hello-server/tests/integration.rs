//! End to end tests: they start the real binary and talk to it over HTTP.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A server process that is killed when the test ends.
struct Server {
    child: Child,
    port: u16,
    /// Keeps the stdout pipe open.
    ///
    /// If the read end of the pipe were closed, the next write by the
    /// server would raise SIGPIPE and kill it, which made the tests flaky.
    _stdout: std::thread::JoinHandle<String>,
    /// Ports that are held busy for the whole test.
    _blockers: Vec<TcpListener>,
}

/// Base ports used by the tests.
///
/// They sit below the operating system's ephemeral port range so that the
/// kernel never hands one of them out to an unrelated connection, which would
/// make the fallback assertions flaky.
const PORTS: [u16; 6] = [18080, 18090, 18100, 18110, 18120, 18130];

impl Server {
    /// Start the binary and wait until it reports the port it listens on.
    fn start(args: &[&str], occupy: &[u16]) -> Server {
        // Occupy the requested ports so the fallback path is exercised. The
        // blockers are held for the whole life of the test, because the
        // server has to find them busy when it binds.
        let blockers: Vec<TcpListener> = occupy
            .iter()
            .map(|&port| {
                TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).unwrap()
            })
            .collect();

        let mut child = Command::new(env!("CARGO_BIN_EXE_hello-server"))
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to start hello-server");

        let stdout = child.stdout.take().expect("no stdout");
        let mut reader = BufReader::new(stdout);

        // The server prints "  Port:       <n>" once it is listening.
        let mut port = None;
        let mut line = String::new();
        while let Ok(n) = reader.read_line(&mut line) {
            if n == 0 {
                break;
            }
            if let Some(rest) = line.trim().strip_prefix("Port:") {
                port = rest.trim().parse::<u16>().ok();
                break;
            }
            line.clear();
        }

        // Keep draining the output in the background so the pipe stays open
        // and the server never blocks on a full pipe buffer.
        let pump = std::thread::spawn(move || {
            let mut collected = String::new();
            let _ = reader.read_to_string(&mut collected);
            collected
        });

        Server {
            child,
            port: port.expect("server did not report a port"),
            _stdout: pump,
            _blockers: blockers,
        }
    }

    /// Perform a raw HTTP/1.0 GET and return the status line plus body.
    fn get(&self, path: &str) -> (String, String) {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port);
        let mut stream = TcpStream::connect(addr).expect("connection refused");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream, "GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").unwrap();
        stream.flush().unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();

        let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
        let status = head.lines().next().unwrap_or("").to_string();
        (status, body.to_string())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn serves_hello_world_on_the_default_port() {
    let port = PORTS[0];
    let server = Server::start(&["--port", &port.to_string()], &[]);

    let (status, body) = server.get("/");
    assert!(status.contains("200 OK"), "unexpected status: {status}");
    assert!(body.contains("Hello, World!"), "unexpected body: {body}");
    assert!(
        body.contains(&port.to_string()),
        "body should mention the port"
    );
}

#[test]
fn falls_back_to_the_next_port_when_the_default_is_busy() {
    let port = PORTS[1];
    let server = Server::start(&["--port", &port.to_string()], &[port]);

    assert_eq!(server.port, port + 1, "should have moved to the next port");

    let (status, body) = server.get("/");
    assert!(status.contains("200 OK"), "unexpected status: {status}");
    assert!(body.contains("Hello, World!"), "unexpected body: {body}");
}

#[test]
fn falls_back_for_a_custom_busy_port_too() {
    let port = PORTS[2];
    let server = Server::start(&["--port", &port.to_string()], &[port]);

    assert_ne!(server.port, port);
    let (status, _) = server.get("/");
    assert!(status.contains("200 OK"), "unexpected status: {status}");
}

#[test]
fn unknown_paths_return_404() {
    let port = PORTS[3];
    let server = Server::start(&["--port", &port.to_string()], &[]);

    let (status, _) = server.get("/does-not-exist");
    assert!(status.contains("404"), "unexpected status: {status}");
}

#[test]
fn health_endpoint_responds() {
    let port = PORTS[4];
    let server = Server::start(&["--port", &port.to_string()], &[]);

    let (status, body) = server.get("/health");
    assert!(status.contains("200 OK"), "unexpected status: {status}");
    assert_eq!(body.trim(), "ok");
}

#[test]
fn server_starts_promptly() {
    let port = PORTS[5];
    let start = Instant::now();
    let server = Server::start(&["--port", &port.to_string()], &[]);
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(server.port, port);
}
