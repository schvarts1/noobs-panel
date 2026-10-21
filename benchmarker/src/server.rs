//! The web server of the benchmarker.

use std::net::IpAddr;
use std::path::PathBuf;

use tiny_http::{Header, Response, Server, StatusCode};

use hello_server::port::BoundPort;

use crate::limits::{cpu_seconds, memory_kb, Limits};
use crate::stress;

/// The response type of the benchmarker.
type BenchResponse = Response<std::io::Cursor<Vec<u8>>>;

/// Run the server until the process is stopped.
pub fn serve(bound: BoundPort, host: IpAddr, directory: PathBuf) -> Result<(), String> {
    let BoundPort {
        listener,
        port,
        requested,
    } = bound;

    let url = hello_server::port::display_url(host, port);

    println!("  Benchmarker is running");
    println!("  ---------------------");
    println!("  URL:        {url}");
    println!("  Port:       {port}");
    if port != requested {
        println!("  Note:       port {requested} was busy, using {port} instead");
    }
    println!("  Stop with:  Ctrl+C");
    println!();

    let server = Server::from_listener(listener, None).map_err(|err| err.to_string())?;

    for request in server.incoming_requests() {
        let path = request.url().to_string();
        let method = request.method().clone();

        let response: BenchResponse = match path.as_str() {
            "/" | "/index.html" => status_page(port),
            "/health" => Response::from_string("ok\n"),
            "/stress/memory" => Response::from_string(stress::memory()),
            "/stress/disk" => Response::from_string(stress::disk(&directory)),
            "/stress/cpu" => Response::from_string(stress::cpu()),
            _ => Response::from_string("404 Not Found\n").with_status_code(StatusCode(404)),
        };

        let status = response.status_code().0;
        let _ = request.respond(response);
        println!("  {method} {path} -> {status}");
    }

    Ok(())
}

/// The page that shows what the instance is using.
fn status_page(port: u16) -> BenchResponse {
    let limits = Limits::read();
    let mut rows = String::new();
    for (name, (soft, hard)) in &limits.values {
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(name),
            escape(soft),
            escape(hard)
        ));
    }

    let cpu = format!("{:.2}", cpu_seconds());
    let body = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Benchmarker</title>
<style>
  :root {{ color-scheme: light dark; }}
  body {{
    margin: 0;
    padding: 1.5rem;
    font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
    background: #101418;
    color: #e8edf2;
  }}
  h1 {{ margin: 0 0 .25rem; font-size: clamp(1.5rem, 5vw, 2.25rem); }}
  p.sub {{ margin: 0 0 1.5rem; opacity: .75; }}
  section {{ margin-bottom: 1.5rem; }}
  table {{ border-collapse: collapse; width: 100%; max-width: 40rem; }}
  th, td {{ text-align: left; padding: .4rem .6rem; border-bottom: 1px solid rgba(255,255,255,.12); }}
  th {{ opacity: .7; font-weight: 600; }}
  code {{ background: rgba(255,255,255,.08); padding: .15em .45em; border-radius: .35em; }}
  a.button {{
    display: inline-block; margin: .25rem .5rem .25rem 0;
    background: rgba(255,255,255,.1); color: inherit; text-decoration: none;
    padding: .5rem .9rem; border-radius: .5rem; border: 1px solid rgba(255,255,255,.15);
  }}
  .warn {{ color: #ffb4a2; }}
</style>
</head>
<body>
<h1>Benchmarker</h1>
<p class="sub">Running on port {port}. This instance exists to check its own limits.</p>

<section>
<h2>What this instance is using</h2>
<table>
  <tr><th>Resource</th><th>Used</th></tr>
  <tr><td>Memory</td><td>{mem} KB</td></tr>
  <tr><td>Processor time</td><td>{cpu} s</td></tr>
</table>
</section>

<section>
<h2>Push against the limits</h2>
<p>Each of these tries to use as much as it can. The panel limits the instance,
so the kernel is the one that says no.</p>
<p>
  <a class="button" href="/stress/memory">Take as much memory as possible</a>
  <a class="button" href="/stress/disk">Write as much disk as possible</a>
  <a class="button" href="/stress/cpu">Use the processor until the limit</a>
</p>
<p class="warn">The processor test ends this process when the limit is reached.
That is the point of it, and the panel will show the instance as stopped.</p>
</section>

<section>
<h2>The limits the kernel gave this process</h2>
<table>
  <tr><th>Limit</th><th>Soft</th><th>Hard</th></tr>
  {rows}
</table>
</section>
</body>
</html>
"#,
        port = port,
        mem = memory_kb(),
        cpu = cpu,
        rows = rows,
    );

    Response::from_string(body).with_header(
        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap(),
    )
}

/// Escape the characters that have a meaning in html.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
