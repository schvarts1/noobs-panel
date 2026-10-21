//! Small helpers around tiny_http: request parsing, cookies and responses.

use std::collections::HashMap;

use tiny_http::{Header, Response, StatusCode};

/// Maximum size of a request body that will be read, in bytes.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// A parsed request.
pub struct WebRequest {
    /// The underlying tiny_http request.
    pub raw: tiny_http::Request,
    /// The path, without the query string.
    pub path: String,
    /// The query string, without the leading `?`.
    pub query: String,
    /// Cookies sent with the request.
    pub cookies: HashMap<String, String>,
    /// Form fields, from the body of a POST or the query string of a GET.
    pub form: HashMap<String, String>,
}

impl WebRequest {
    /// Wrap a tiny_http request and parse it.
    pub fn new(mut raw: tiny_http::Request) -> WebRequest {
        let url = raw.url().to_string();
        let (path, query) = match url.split_once('?') {
            Some((path, query)) => (path.to_string(), query.to_string()),
            None => (url.clone(), String::new()),
        };

        let cookies = parse_cookies(&raw);
        let mut form = parse_query(&query);

        // A POST body carries the form fields.
        if raw.method() == &tiny_http::Method::Post {
            let content_type = raw
                .headers()
                .iter()
                .find(|h| h.field.equiv("Content-Type"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();
            if content_type.starts_with("application/x-www-form-urlencoded") {
                let mut body = String::new();
                let reader = raw.as_reader();
                let mut buffer = [0u8; 4096];
                // Read at most one byte more than the limit so an oversized
                // body can be detected and rejected.
                while body.len() <= MAX_BODY_BYTES {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => body.push_str(&String::from_utf8_lossy(&buffer[..count])),
                        Err(_) => break,
                    }
                }
                form.extend(parse_query(&body));
            }
        }

        WebRequest {
            raw,
            path,
            query,
            cookies,
            form,
        }
    }

    /// The request method as a plain string.
    pub fn method(&self) -> &str {
        self.raw.method().as_str()
    }

    /// The value of a cookie, if it was sent.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(|value| value.as_str())
    }

    /// The value of a form field.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.form.get(name).map(|value| value.as_str())
    }

    /// A form field, trimmed.
    pub fn trimmed_field(&self, name: &str) -> String {
        self.field(name).unwrap_or_default().trim().to_string()
    }

    /// A form field parsed as a number.
    pub fn number_field(&self, name: &str) -> Option<i64> {
        self.trimmed_field(name).parse::<i64>().ok()
    }

    /// The path split into its non empty segments.
    pub fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }
}

/// Parse the `Cookie` header into a map.
fn parse_cookies(request: &tiny_http::Request) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    for header in request.headers() {
        if !header.field.equiv("Cookie") {
            continue;
        }
        for part in header.value.as_str().split(';') {
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            cookies.insert(name.trim().to_string(), value.trim().to_string());
        }
    }
    cookies
}

/// Parse an url encoded string into a map.
fn parse_query(query: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (name, value) = match pair.split_once('=') {
            Some((name, value)) => (name, value),
            None => (pair, ""),
        };
        fields.insert(url_decode(name), url_decode(value));
    }
    fields
}

/// Percent decode a string, treating `+` as a space.
fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &input[index + 1..index + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Escape text so it can be embedded in HTML.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

/// Build an HTML response.
pub fn html(status: u16, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(
            Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap(),
        )
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
}

/// Build a response with a body of json.
pub fn json(status: u16, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(
            Header::from_bytes(
                &b"Content-Type"[..],
                &b"application/json; charset=utf-8"[..],
            )
            .unwrap(),
        )
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
}

/// Build a redirect response.
pub fn redirect(location: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string("")
        .with_status_code(StatusCode(303))
        .with_header(Header::from_bytes(&b"Location"[..], location.as_bytes()).unwrap())
}

/// Build a response that sets a cookie.
pub fn html_with_cookie(
    status: u16,
    body: String,
    name: &str,
    value: &str,
    max_age: i64,
    http_only: bool,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut cookie = format!("{name}={value}; Path=/; SameSite=Lax; Max-Age={max_age}");
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    let response = html(status, body);
    response.with_header(Header::from_bytes(&b"Set-Cookie"[..], cookie.as_bytes()).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_special_characters() {
        assert_eq!(escape("<script>"), "&lt;script&gt;");
        assert_eq!(escape("a & b"), "a &amp; b");
        assert_eq!(escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(escape("it's"), "it&#39;s");
        assert_eq!(escape("plain"), "plain");
    }

    #[test]
    fn decodes_percent_encoding() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("a+b"), "a b");
        assert_eq!(url_decode("100%25"), "100%");
        assert_eq!(url_decode("bad%zz"), "bad%zz");
    }

    #[test]
    fn parses_query_strings() {
        let fields = parse_query("name=alice&role=admin&empty=");
        assert_eq!(fields.get("name").map(String::as_str), Some("alice"));
        assert_eq!(fields.get("role").map(String::as_str), Some("admin"));
        assert_eq!(fields.get("empty").map(String::as_str), Some(""));
    }

    #[test]
    fn parses_query_strings_with_encoded_values() {
        let fields = parse_query("msg=hello%20world%21");
        assert_eq!(fields.get("msg").map(String::as_str), Some("hello world!"));
    }
}
