//! End to end tests for the panel: a real server, a real database and a real
//! HTTP client driving the whole flow.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A panel process that is stopped when the test ends.
struct Panel {
    child: Child,
    port: u16,
    /// Keeps the stdout pipe open so the server is not killed by SIGPIPE.
    _stdout: std::thread::JoinHandle<String>,
}

/// A response from the panel.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    /// The value of a header.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The value of the session cookie, if the response sets one.
    fn cookie(&self, name: &str) -> Option<String> {
        for (key, value) in &self.headers {
            if key.eq_ignore_ascii_case("set-cookie") {
                let mut parts = value.split(';');
                let pair = parts.next()?;
                let (cookie_name, cookie_value) = pair.split_once('=')?;
                if cookie_name == name {
                    return Some(cookie_value.to_string());
                }
            }
        }
        None
    }
}

/// A very small HTTP client that keeps the session cookie.
#[derive(Clone)]
struct Client {
    port: u16,
    cookie: Option<String>,
    /// The csrf cookie that the panel sets for forms shown before signing in.
    csrf_cookie: Option<String>,
}

impl Client {
    /// A client without a session.
    fn new(port: u16) -> Client {
        Client {
            port,
            cookie: None,
            csrf_cookie: None,
        }
    }

    /// Send a request and return the reply.
    fn request(&mut self, method: &str, path: &str, body: Option<&str>) -> Reply {
        let mut stream =
            TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port))
                .expect("cannot reach the panel");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();

        let payload = body.unwrap_or_default();
        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
        if let Some(cookie) = &self.cookie {
            request.push_str(&format!("Cookie: {cookie}\r\n"));
        }
        if let Some(cookie) = &self.csrf_cookie {
            request.push_str(&format!("Cookie: {cookie}\r\n"));
        }
        if body.is_some() {
            request.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
            request.push_str(&format!("Content-Length: {}\r\n", payload.len()));
        }
        request.push_str("\r\n");
        request.push_str(payload);

        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).into_owned();

        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let mut lines = head.lines();
        let status_line = lines.next().unwrap_or_default();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or(0);
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .collect();

        // Keep the cookies for the next request.
        for (key, value) in &headers {
            if !key.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            let Some((cookie_name, cookie_value)) = value
                .split(';')
                .next()
                .and_then(|pair| pair.split_once('='))
            else {
                continue;
            };
            match cookie_name {
                "noobs_session" if !cookie_value.is_empty() => {
                    self.cookie = Some(format!("noobs_session={cookie_value}"));
                }
                "noobs_csrf" if !cookie_value.is_empty() => {
                    self.csrf_cookie = Some(format!("noobs_csrf={cookie_value}"));
                }
                _ => {}
            }
        }

        Reply {
            status,
            headers,
            body: body.to_string(),
        }
    }

    /// Send a GET request.
    fn get(&mut self, path: &str) -> Reply {
        self.request("GET", path, None)
    }

    /// Send a POST request with a form body.
    fn post(&mut self, path: &str, body: &str) -> Reply {
        self.request("POST", path, Some(body))
    }

    /// The csrf token that is embedded in a page.
    fn csrf(&mut self, path: &str) -> String {
        let reply = self.get(path);
        extract_csrf(&reply.body)
    }
}

/// Pull the csrf token out of a page.
fn extract_csrf(html: &str) -> String {
    let marker = "name=\"_csrf\" value=\"";
    let start = html
        .find(marker)
        .unwrap_or_else(|| panic!("no csrf token in the page:\n{html}"))
        + marker.len();
    let end = html[start..].find('"').expect("unterminated csrf token") + start;
    html[start..end].to_string()
}

/// The id of an account in the owner list of a page.
///
/// The name of the account is shown after the value of the option.
fn extract_owner(html: &str, username: &str) -> String {
    let mut search = 0;
    while let Some(offset) = html[search..].find("<option value=\"") {
        let start = search + offset + "<option value=\"".len();
        let end = html[start..].find('"').expect("unterminated owner") + start;
        let id = html[start..end].to_string();
        let rest = &html[end..];
        let name_at = rest.find('>').expect("unterminated option") + 1;
        let name_end = rest[name_at..].find('<').expect("unterminated option") + name_at;
        if rest[name_at..name_end].trim() == username {
            return id;
        }
        search = end;
    }
    panic!("no account {username} in the owner list");
}

/// Start a panel on a free port with a fresh database.
fn start_panel() -> Panel {
    // Every test needs its own data directory, so the database of one test
    // cannot be seen by another.
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let data_dir =
        std::env::temp_dir().join(format!("noobs-panel-test-{}-{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_dir);
    std::fs::create_dir_all(&data_dir).unwrap();

    // Ask the operating system for a free port by binding one and letting go
    // of it again straight away.
    let probe =
        std::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let mut child = Command::new(env!("CARGO_BIN_EXE_noobs-panel"))
        .arg("--port")
        .arg(port.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--data-dir")
        .arg(&data_dir)
        // Every test gets its own range of instance ports.
        .env(
            "NOOBS_PANEL_INSTANCE_PORTS",
            format!("{}-{}", 19000 + unique * 10, 19009 + unique * 10),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start the panel");

    let stdout = child.stdout.take().expect("no stdout");
    let pump = std::thread::spawn(move || {
        let mut collected = String::new();
        let mut reader = std::io::BufReader::new(stdout);
        let _ = reader.read_to_string(&mut collected);
        collected
    });

    // Wait for the panel to accept connections.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    Panel {
        child,
        port,
        _stdout: pump,
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn signup_login_and_reach_the_dashboard() {
    let panel = start_panel();
    let mut client = Client::new(panel.port);

    // The signup page is reachable and carries a csrf token.
    let page = client.get("/signup");
    assert_eq!(page.status, 200);
    assert!(page.body.contains("Create account"));
    let csrf = extract_csrf(&page.body);
    assert!(!csrf.is_empty());

    // Registering without the csrf token is refused.
    let refused = client.post(
        "/signup",
        "username=alice&email=alice@example.com&password=hunter2hunter2",
    );
    assert_eq!(refused.status, 200);
    assert!(refused.body.contains("not submitted correctly"));

    // Registering with it works.
    let created = client.post(
        "/signup",
        &format!("_csrf={csrf}&username=alice&email=alice@example.com&password=hunter2hunter2"),
    );
    assert_eq!(created.status, 303);
    assert_eq!(created.header("Location"), Some("/login?created=1"));

    // The account can sign in.
    let login_page = client.get("/login");
    let login_csrf = extract_csrf(&login_page.body);
    let signed_in = client.post(
        "/login",
        &format!("_csrf={login_csrf}&username=alice&password=hunter2hunter2"),
    );
    assert_eq!(signed_in.status, 303);
    assert_eq!(signed_in.header("Location"), Some("/dashboard"));
    assert!(signed_in.cookie("noobs_session").is_some());

    // The dashboard is now reachable and greets the user.
    let dashboard = client.get("/dashboard");
    assert_eq!(dashboard.status, 200);
    assert!(dashboard.body.contains("Dashboard"));
    assert!(dashboard.body.contains("alice"));
}

#[test]
fn the_first_account_becomes_an_administrator() {
    let panel = start_panel();
    let mut client = Client::new(panel.port);

    let csrf = client.csrf("/signup");
    client.post(
        "/signup",
        &format!("_csrf={csrf}&username=root&email=root@example.com&password=hunter2hunter2"),
    );

    let login_csrf = client.csrf("/login");
    client.post(
        "/login",
        &format!("_csrf={login_csrf}&username=root&password=hunter2hunter2"),
    );

    // The admin area is reachable for the first account.
    let admin = client.get("/admin");
    assert_eq!(admin.status, 200);
    assert!(admin.body.contains("Administration"));
    assert!(admin.body.contains("root"));
}

#[test]
fn a_second_account_is_not_an_administrator() {
    let panel = start_panel();

    // The first account becomes an administrator.
    let mut admin = Client::new(panel.port);
    let csrf = admin.csrf("/signup");
    admin.post(
        "/signup",
        &format!("_csrf={csrf}&username=root&email=root@example.com&password=hunter2hunter2"),
    );
    let login_csrf = admin.csrf("/login");
    admin.post(
        "/login",
        &format!("_csrf={login_csrf}&username=root&password=hunter2hunter2"),
    );

    // A second account is a plain user.
    let mut plain = Client::new(panel.port);
    let csrf = plain.csrf("/signup");
    plain.post(
        "/signup",
        &format!("_csrf={csrf}&username=bob&email=bob@example.com&password=hunter2hunter2"),
    );
    let login_csrf = plain.csrf("/login");
    plain.post(
        "/login",
        &format!("_csrf={login_csrf}&username=bob&password=hunter2hunter2"),
    );

    // The admin area is closed for a plain user.
    let admin = plain.get("/admin");
    assert_eq!(admin.status, 403);
}

#[test]
fn a_wrong_password_does_not_sign_in() {
    let panel = start_panel();
    let mut client = Client::new(panel.port);

    let csrf = client.csrf("/signup");
    client.post(
        "/signup",
        &format!("_csrf={csrf}&username=alice&email=alice@example.com&password=hunter2hunter2"),
    );

    let login_csrf = client.csrf("/login");
    let reply = client.post(
        "/login",
        &format!("_csrf={login_csrf}&username=alice&password=wrongpassword"),
    );
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains("Wrong username or password"));
    assert!(reply.cookie("noobs_session").is_none());
}

#[test]
fn a_signed_out_visitor_is_sent_to_the_login_page() {
    let panel = start_panel();
    let mut client = Client::new(panel.port);

    let reply = client.get("/dashboard");
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains("Please sign in to continue"));
}

#[test]
fn an_instance_can_be_created_and_is_limited_by_the_account_quota() {
    let panel = start_panel();

    // The administrator is the first account, so alice is a regular user.
    let mut admin = admin_client(&panel);

    let mut client = Client::new(panel.port);
    let csrf = client.csrf("/signup");
    client.post(
        "/signup",
        &format!("_csrf={csrf}&username=alice&email=alice@example.com&password=hunter2hunter2"),
    );
    let login_csrf = client.csrf("/login");
    client.post(
        "/login",
        &format!("_csrf={login_csrf}&username=alice&password=hunter2hunter2"),
    );

    // A regular user may not open the create page.
    let page = client.get("/instances/new");
    assert_eq!(page.status, 403);

    // The administrator creates the instance for the account.
    let page = admin.get("/instances/new");
    assert_eq!(page.status, 200);
    assert!(page.body.contains("New instance"));
    let create_csrf = extract_csrf(&page.body);

    // The form asks which account the instance is for.
    let owner = extract_owner(&page.body, "alice");
    assert!(!owner.is_empty());

    // A limit larger than the account has is refused.
    let too_big = admin.post(
        "/instances/new",
        &format!(
            "_csrf={create_csrf}&name=big-web&owner={owner}&preset=hello-world&memory=99999&cpu=30&disk=256"
        ),
    );
    assert_eq!(too_big.status, 200);
    assert!(too_big.body.contains("enough resources left"));

    // A reasonable limit is accepted.
    let created = admin.post(
        "/instances/new",
        &format!(
            "_csrf={create_csrf}&name=my-web&owner={owner}&preset=hello-world&memory=128&cpu=30&disk=256"
        ),
    );
    assert_eq!(created.status, 303);
    assert!(created
        .header("Location")
        .expect("no location")
        .starts_with("/instances/"));

    // The instance shows up on the dashboard of the account it was made for.
    let dashboard = client.get("/dashboard");
    assert!(dashboard.body.contains("my-web"));
}

#[test]
fn an_administrator_can_change_the_limits_of_an_account() {
    let panel = start_panel();

    let mut admin = Client::new(panel.port);
    let csrf = admin.csrf("/signup");
    admin.post(
        "/signup",
        &format!("_csrf={csrf}&username=root&email=root@example.com&password=hunter2hunter2"),
    );
    let login_csrf = admin.csrf("/login");
    admin.post(
        "/login",
        &format!("_csrf={login_csrf}&username=root&password=hunter2hunter2"),
    );

    // Create a second account.
    let mut plain = Client::new(panel.port);
    let csrf = plain.csrf("/signup");
    plain.post(
        "/signup",
        &format!("_csrf={csrf}&username=bob&email=bob@example.com&password=hunter2hunter2"),
    );

    // The admin page of the account lists the current limits.
    let user_page = admin.get("/admin/users/2");
    assert_eq!(user_page.status, 200);
    assert!(user_page.body.contains("bob"));
    let admin_csrf = extract_csrf(&user_page.body);

    // Raising the limits is possible.
    let saved = admin.post(
        "/admin/users/2/limits",
        &format!("_csrf={admin_csrf}&memory=1024&cpu=600&disk=4096&instances=9"),
    );
    assert_eq!(saved.status, 200);
    assert!(saved.body.contains("The limits were saved"));
    assert!(saved.body.contains("1024"));
}

/// Sign in as the first account, which is the administrator.
fn admin_client(panel: &Panel) -> Client {
    let mut client = Client::new(panel.port);
    let csrf = client.csrf("/signup");
    client.post(
        "/signup",
        &format!("_csrf={csrf}&username=root&email=root@example.com&password=supersecret1"),
    );
    let csrf = client.csrf("/login");
    client.post(
        "/login",
        &format!("_csrf={csrf}&username=root&password=supersecret1"),
    );
    client
}

/// An administrator creates an instance for an account and returns its id.
fn create_instance(admin: &mut Client, owner: &str, name: &str, preset: &str) -> String {
    let page = admin.get("/instances/new");
    assert_eq!(page.status, 200);
    let csrf = extract_csrf(&page.body);
    let created = admin.post(
        "/instances/new",
        &format!(
            "_csrf={csrf}&name={name}&owner={owner}&preset={preset}&memory=128&cpu=30&disk=256"
        ),
    );
    assert_eq!(created.status, 303);
    let location = created.header("Location").expect("no location").to_string();
    location.trim_start_matches("/instances/").to_string()
}

/// Sign in as a fresh account and return a client that has a session.
fn signed_in_client(panel: &Panel, username: &str) -> Client {
    let mut client = Client::new(panel.port);
    let csrf = client.csrf("/signup");
    client.post(
        "/signup",
        &format!(
            "_csrf={csrf}&username={username}&email={username}@example.com&password=supersecret1"
        ),
    );
    let csrf = client.csrf("/login");
    client.post(
        "/login",
        &format!("_csrf={csrf}&username={username}&password=supersecret1"),
    );
    client
}

#[test]
fn an_instance_is_started_serves_requests_and_can_be_stopped() {
    let panel = start_panel();
    let mut admin = admin_client(&panel);
    let mut client = signed_in_client(&panel, "alice");

    // The administrator creates the instance for the account.
    let owner = extract_owner(&admin.get("/instances/new").body, "alice");
    let id = create_instance(&mut admin, &owner, "my-web", "hello-world");

    // The instance shows up on the dashboard of the account it was made for.
    let dashboard = client.get("/dashboard");
    assert!(dashboard.body.contains("my-web"));
    assert_eq!(extract_id(&dashboard.body), id);

    // Start it.
    let page = client.get(&format!("/instances/{id}"));
    let csrf = extract_csrf(&page.body);
    let started = client.post(&format!("/instances/{id}/start"), &format!("_csrf={csrf}"));
    assert_eq!(started.status, 303);

    // The page reports that it is running and shows the port.
    let page = client.get(&format!("/instances/{id}"));
    assert!(page.body.contains("Running"), "instance should be running");

    // The instance itself answers requests.
    let port = extract_port(&page.body);
    let mut stream = TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        .expect("the instance should be listening");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    assert!(
        answer.contains("Hello, World!"),
        "unexpected answer from the instance: {answer}"
    );

    // Stop it again.
    let page = client.get(&format!("/instances/{id}"));
    let csrf = extract_csrf(&page.body);
    let stopped = client.post(&format!("/instances/{id}/stop"), &format!("_csrf={csrf}"));
    assert_eq!(stopped.status, 303);

    let page = client.get(&format!("/instances/{id}"));
    assert!(page.body.contains("Stopped"), "instance should be stopped");
}

#[test]
fn an_instance_can_be_deleted() {
    let panel = start_panel();
    let mut admin = admin_client(&panel);
    let mut client = signed_in_client(&panel, "alice");

    let owner = extract_owner(&admin.get("/instances/new").body, "alice");
    let id = create_instance(&mut admin, &owner, "my-web", "hello-world");

    let dashboard = client.get("/dashboard");
    assert_eq!(extract_id(&dashboard.body), id);

    let page = client.get(&format!("/instances/{id}"));
    let csrf = extract_csrf(&page.body);
    let deleted = client.post(&format!("/instances/{id}/delete"), &format!("_csrf={csrf}"));
    assert_eq!(deleted.status, 303);

    let dashboard = client.get("/dashboard");
    assert!(
        !dashboard.body.contains("my-web"),
        "the instance should be gone"
    );
}

#[test]
fn a_user_cannot_touch_the_instance_of_somebody_else() {
    let panel = start_panel();
    let mut admin = admin_client(&panel);

    // The administrator creates an instance for Alice.
    let mut alice = signed_in_client(&panel, "alice");
    let owner = extract_owner(&admin.get("/instances/new").body, "alice");
    let id = create_instance(&mut admin, &owner, "alice-web", "hello-world");
    let dashboard = alice.get("/dashboard");
    assert_eq!(extract_id(&dashboard.body), id);

    // Bob cannot see it, and cannot stop it either.
    let mut bob = signed_in_client(&panel, "bob");
    let page = bob.get(&format!("/instances/{id}"));
    assert_eq!(page.status, 403);
    assert!(page.body.contains("does not belong to you"));

    let csrf = bob.csrf("/dashboard");
    let stopped = bob.post(&format!("/instances/{id}/stop"), &format!("_csrf={csrf}"));
    assert_eq!(stopped.status, 403);
}

/// The id of the first instance on a page.
///
/// The link to the create form also starts with `/instances/`, so only
/// numeric ids are accepted.
fn extract_id(html: &str) -> String {
    let marker = "href=\"/instances/";
    let mut search = 0;
    while let Some(offset) = html[search..].find(marker) {
        let start = search + offset + marker.len();
        let end = html[start..].find('"').expect("unterminated id") + start;
        let candidate = &html[start..end];
        if candidate.chars().all(|c| c.is_ascii_digit()) {
            return candidate.to_string();
        }
        search = end;
    }
    panic!("no instance on the page");
}

/// The port of the instance on a page.
fn extract_port(html: &str) -> u16 {
    let marker = "http://127.0.0.1:";
    let start = html.find(marker).expect("no port on the page") + marker.len();
    let end = html[start..]
        .find(|c: char| !c.is_ascii_digit())
        .expect("unterminated port")
        + start;
    html[start..end].parse().expect("port is not a number")
}

#[test]
fn a_form_still_works_when_a_second_page_was_opened() {
    let panel = start_panel();
    let mut client = Client::new(panel.port);

    // The registration form is opened twice, as it happens when the page is
    // reloaded or opened in a second tab.
    let first = client.csrf("/signup");
    let second = client.csrf("/signup");
    assert_eq!(first, second, "the form should use the same token");

    let reply = client.post(
        "/signup",
        &format!("_csrf={first}&username=alice&email=alice@example.com&password=supersecret1"),
    );
    assert_eq!(reply.status, 303);
}

#[test]
fn the_form_can_be_submitted_again_after_a_failed_attempt() {
    let panel = start_panel();
    let mut client = Client::new(panel.port);

    // A token the panel does not know is refused.
    let reply = client.post(
        "/signup",
        "_csrf=deadbeef&username=alice&email=alice@example.com&password=supersecret1",
    );
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains("not submitted correctly"));

    // The page that came back has to carry a token that works, so the user
    // can simply press the button again.
    let token = extract_csrf(&reply.body);
    let reply = client.post(
        "/signup",
        &format!("_csrf={token}&username=alice&email=alice@example.com&password=supersecret1"),
    );
    assert_eq!(reply.status, 303);
}

#[test]
fn the_form_keeps_working_after_an_ordinary_error() {
    let panel = start_panel();
    let mut client = Client::new(panel.port);

    // The first account is registered.
    let token = client.csrf("/signup");
    let reply = client.post(
        "/signup",
        &format!("_csrf={token}&username=alice&email=alice@example.com&password=supersecret1"),
    );
    assert_eq!(reply.status, 303);

    // A second client tries to use the same name.
    let mut other = Client::new(panel.port);
    let token = other.csrf("/signup");
    let reply = other.post(
        "/signup",
        &format!("_csrf={token}&username=alice&email=other@example.com&password=supersecret1"),
    );
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains("already taken"));

    // The form that came back has to work, so a different name can be used.
    let token = extract_csrf(&reply.body);
    let reply = other.post(
        "/signup",
        &format!("_csrf={token}&username=bob&email=bob@example.com&password=supersecret1"),
    );
    assert_eq!(reply.status, 303);
}

#[test]
fn a_forged_csrf_cookie_is_refused() {
    let panel = start_panel();
    let mut client = Client::new(panel.port);

    // A cookie that was put in the browser by somebody else, with a token
    // that looks like one of ours.
    let forged = "ab".repeat(32);
    client.csrf_cookie = Some(format!("noobs_csrf={forged}"));

    let reply = client.post(
        "/signup",
        &format!("_csrf={forged}&username=alice&email=alice@example.com&password=supersecret1"),
    );
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains("not submitted correctly"));
}

#[test]
fn only_an_administrator_can_create_an_instance() {
    let panel = start_panel();
    let _ = admin_client(&panel);
    let mut alice = signed_in_client(&panel, "alice");

    // The page is closed.
    let page = alice.get("/instances/new");
    assert_eq!(page.status, 403);
    assert!(page.body.contains("do not have permission"));

    // And so is the form.
    let csrf = alice.csrf("/dashboard");
    let refused = alice.post(
        "/instances/new",
        &format!("_csrf={csrf}&name=sneaky&owner=2&preset=hello-world&memory=64&cpu=30&disk=64"),
    );
    assert_eq!(refused.status, 403);

    // Nothing was created.
    let dashboard = alice.get("/dashboard");
    assert!(!dashboard.body.contains("sneaky"));
}

#[test]
fn an_administrator_can_hand_an_instance_to_an_account() {
    let panel = start_panel();
    let mut admin = admin_client(&panel);
    let mut alice = signed_in_client(&panel, "alice");
    let mut bob = signed_in_client(&panel, "bob");

    let owner = extract_owner(&admin.get("/instances/new").body, "alice");
    let id = create_instance(&mut admin, &owner, "my-web", "hello-world");

    // The instance belongs to Alice.
    let dashboard = alice.get("/dashboard");
    assert!(dashboard.body.contains("my-web"));
    let dashboard = bob.get("/dashboard");
    assert!(!dashboard.body.contains("my-web"));

    // The administrator hands it to Bob.
    let page = admin.get(&format!("/instances/{id}"));
    let csrf = extract_csrf(&page.body);
    let bob_id = extract_owner(&page.body, "bob");
    let handed = admin.post(
        &format!("/instances/{id}/owner"),
        &format!("_csrf={csrf}&owner={bob_id}"),
    );
    assert_eq!(handed.status, 303);

    // Now it belongs to Bob.
    let dashboard = bob.get("/dashboard");
    assert!(dashboard.body.contains("my-web"));
    let dashboard = alice.get("/dashboard");
    assert!(!dashboard.body.contains("my-web"));
}

#[test]
fn a_regular_user_cannot_hand_an_instance_over() {
    let panel = start_panel();
    let mut admin = admin_client(&panel);
    let mut alice = signed_in_client(&panel, "alice");

    let owner = extract_owner(&admin.get("/instances/new").body, "alice");
    let id = create_instance(&mut admin, &owner, "my-web", "hello-world");

    let page = alice.get(&format!("/instances/{id}"));
    let csrf = extract_csrf(&page.body);
    let refused = alice.post(
        &format!("/instances/{id}/owner"),
        &format!("_csrf={csrf}&owner=1"),
    );
    assert_eq!(refused.status, 403);
}

#[test]
fn an_instance_cannot_be_handed_to_an_account_without_room() {
    let panel = start_panel();
    let mut admin = admin_client(&panel);
    let _alice = signed_in_client(&panel, "alice");

    let owner = extract_owner(&admin.get("/instances/new").body, "alice");
    let id = create_instance(&mut admin, &owner, "my-web", "hello-world");

    // Alice has room for one more instance, so the administrator takes it
    // away from her and hands it to her again.
    let page = admin.get(&format!("/instances/{id}"));
    let csrf = extract_csrf(&page.body);

    // An account that does not exist is refused.
    let refused = admin.post(
        &format!("/instances/{id}/owner"),
        &format!("_csrf={csrf}&owner=999"),
    );
    assert_eq!(refused.status, 200);
    assert!(refused.body.contains("does not exist"));
}

#[test]
fn an_instance_runs_the_benchmarker_preset() {
    let panel = start_panel();
    let mut admin = admin_client(&panel);
    let mut alice = signed_in_client(&panel, "alice");

    let owner = extract_owner(&admin.get("/instances/new").body, "alice");
    let id = create_instance(&mut admin, &owner, "bench", "benchmarker");

    // The page says which preset it runs.
    let page = alice.get(&format!("/instances/{id}"));
    assert!(page.body.contains("Benchmarker"));

    // And it really runs the benchmarker.
    let page = alice.get(&format!("/instances/{id}"));
    let csrf = extract_csrf(&page.body);
    let started = alice.post(&format!("/instances/{id}/start"), &format!("_csrf={csrf}"));
    assert_eq!(started.status, 303);

    let page = alice.get(&format!("/instances/{id}"));
    assert!(page.body.contains("Running"));
    let port = extract_port(&page.body);

    // The benchmarker answers and shows the limits the kernel gave it.
    let body = http_get(port, "/");
    assert!(body.contains("Benchmarker"));
    assert!(body.contains("Max data size"));
}

#[test]
fn an_instance_needs_a_known_preset() {
    let panel = start_panel();
    let mut admin = admin_client(&panel);
    let _ = signed_in_client(&panel, "alice");

    let page = admin.get("/instances/new");
    let csrf = extract_csrf(&page.body);
    let owner = extract_owner(&page.body, "alice");

    let refused = admin.post(
        "/instances/new",
        &format!("_csrf={csrf}&name=my-web&owner={owner}&preset=nope&memory=64&cpu=30&disk=64"),
    );
    assert_eq!(refused.status, 200);
    assert!(refused.body.contains("choose a preset"));
}

/// Ask a server on a port for a page.
fn http_get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        .expect("the instance should be listening");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").as_bytes())
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}
