//! The HTTP server itself.

use std::net::IpAddr;

use tiny_http::{Header, Response, Server, StatusCode};

use crate::port::BoundPort;
use crate::site;

/// The body served for the hello world page.
const HELLO_BODY: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
<title>Hello, World!</title>
<style>
  :root { color-scheme: light dark; }
  body {
    margin: 0;
    min-height: 100vh;
    display: grid;
    place-items: center;
    font-family: system-ui, -apple-system, \"Segoe UI\", Roboto, sans-serif;
    background: #101418;
    color: #e8edf2;
  }
  main { text-align: center; padding: 2rem; }
  h1 {
    margin: 0 0 .5rem;
    font-size: clamp(2rem, 8vw, 4rem);
    letter-spacing: -0.02em;
  }
  p { margin: 0; opacity: .75; }
  code {
    background: rgba(255,255,255,.08);
    padding: .15em .45em;
    border-radius: .35em;
    font-size: .95em;
  }
</style>
</head>
<body>
<main>
  <h1>Hello, World!</h1>
  <p>Served by a Rust web server on port <code>{port}</code>.</p>
</main>
</body>
</html>
";

/// The response type produced by [`hello_response`].
type HtmlResponse = Response<std::io::Cursor<Vec<u8>>>;

/// Build the hello world response for a given port.
fn hello_response(port: u16) -> HtmlResponse {
    let body = HELLO_BODY.replace("{port}", &port.to_string());
    Response::from_string(body).with_header(
        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap(),
    )
}

/// Run the server until the process is stopped.
pub fn serve(
    bound: BoundPort,
    host: IpAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    serve_with_root(bound, host, None)
}

/// Run the server until the process is stopped, serving the files of a site.
///
/// When no directory is given, the built in hello world page is served.
pub fn serve_with_root(
    bound: BoundPort,
    host: IpAddr,
    root: Option<std::path::PathBuf>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let BoundPort {
        listener,
        port,
        requested,
    } = bound;

    let url = crate::port::display_url(host, port);

    println!("  Hello, World! server is running");
    println!("  ---------------------------------");
    println!("  URL:        {url}");
    println!("  Port:       {port}");
    if port != requested {
        println!("  Note:       port {requested} was busy, using {port} instead");
    }
    println!("  Stop with:  Ctrl+C");
    println!();

    let server = Server::from_listener(listener, None)?;

    for request in server.incoming_requests() {
        let path = request.url().to_string();
        let method = request.method().clone();

        // A file of the site is served when there is one.
        let served = root.as_ref().and_then(|root| site::serve_file(root, &path));

        let response: HtmlResponse = if let Some(response) = served {
            response
        } else if path == "/health" {
            Response::from_string("ok\n")
        } else if path == "/" || path == "/index.html" {
            hello_response(port)
        } else {
            Response::from_string("404 Not Found\n").with_status_code(StatusCode(404))
        };

        let status = response.status_code().0;
        let _ = request.respond(response);
        println!("  {method} {path} -> {status}");
    }

    Ok(())
}
