//! Serving the files of a site.
//!
//! When the server is given a directory, the files in it are served instead
//! of the built in hello world page. The user of an instance can put their
//! own html, css and javascript there.

use std::path::{Path, PathBuf};

use tiny_http::{Header, Response, StatusCode};

/// The response type of the server.
type FileResponse = Response<std::io::Cursor<Vec<u8>>>;

/// The largest file that is served.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The name of the file that is served for the root of the site.
const INDEX: &str = "index.html";

/// The content type of a file, based on its name.
fn content_type(name: &str) -> &'static str {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// The file a path asks for.
///
/// The name is cleaned up, so a path cannot reach outside the directory.
fn resolve(root: &Path, path: &str) -> Option<PathBuf> {
    let name = path.trim_start_matches('/');
    let name = if name.is_empty() { INDEX } else { name };

    // Only the name of the file is used, so a path with directories in it
    // cannot reach outside the directory of the site.
    let name = Path::new(name)
        .file_name()
        .map(|value| value.to_string_lossy().to_string())?;

    if name.is_empty() || name == "." || name == ".." {
        return None;
    }

    let candidate = root.join(&name);
    if candidate.is_file() {
        return Some(candidate);
    }

    // The root of the site is the index file.
    if name == INDEX {
        return None;
    }

    None
}

/// Serve a file of the site.
///
/// Returns `None` when there is no such file, so the caller can fall back
/// to the hello world page.
///
/// The file gets a mark that says when it was changed, so a browser that
/// has it does not have to fetch it again. When the browser sends the mark
/// it has, the answer is that nothing changed.
pub fn serve_file(root: &Path, path: &str, known: Option<&str>) -> Option<FileResponse> {
    let file = resolve(root, path)?;

    let Ok(metadata) = file.metadata() else {
        return None;
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Some(
            Response::from_string("The file is too large.\n").with_status_code(StatusCode(413)),
        );
    }

    // The mark of the file is made of the moment it was changed and its
    // size, so a file that is changed gets a new mark.
    let mark = file_mark(&metadata);
    if known.is_some_and(|value| value == mark) {
        // The browser has this file already, and nothing changed.
        return Some(
            Response::from_string("")
                .with_status_code(StatusCode(304))
                .with_header(Header::from_bytes(&b"ETag"[..], mark.as_bytes()).unwrap())
                .with_header(
                    Header::from_bytes(&b"Cache-Control"[..], CACHE_CONTROL.as_bytes()).unwrap(),
                ),
        );
    }

    let Ok(content) = std::fs::read(&file) else {
        return None;
    };

    let name = file.to_string_lossy().to_string();
    Some(
        Response::from_data(content)
            .with_header(
                Header::from_bytes(&b"Content-Type"[..], content_type(&name).as_bytes()).unwrap(),
            )
            .with_header(Header::from_bytes(&b"ETag"[..], mark.as_bytes()).unwrap())
            .with_header(
                Header::from_bytes(&b"Last-Modified"[..], last_modified(&metadata).as_bytes())
                    .unwrap(),
            )
            .with_header(
                Header::from_bytes(&b"Cache-Control"[..], CACHE_CONTROL.as_bytes()).unwrap(),
            ),
    )
}

/// What the browser is told about the files of a site.
///
/// The browser has to ask again every time, so a change is seen at once,
/// but it may keep the file when the answer says nothing changed.
const CACHE_CONTROL: &str = "no-cache";

/// The mark of a file, made of the moment it was changed and its size.
fn file_mark(metadata: &std::fs::Metadata) -> String {
    let changed = metadata
        .modified()
        .ok()
        .and_then(|when| when.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|when| when.as_secs())
        .unwrap_or(0);
    format!("\"{changed:x}-{:x}\"", metadata.len())
}

/// The moment a file was changed, in the format of the http date.
fn last_modified(metadata: &std::fs::Metadata) -> String {
    let Ok(changed) = metadata.modified() else {
        return String::new();
    };
    let Ok(when) = changed.duration_since(std::time::UNIX_EPOCH) else {
        return String::new();
    };

    // The days since the start of the week, and the days since the start
    // of the year, of the day the file was changed.
    let days = when.as_secs() / 86_400;
    let weekday = (days + 4) % 7;
    let year_start = days / 365;
    let day_of_year = days % 365;
    let month = (day_of_year / 30).min(11);
    let day = day_of_year % 30 + 1;

    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[weekday as usize],
        day,
        MONTHS[month as usize],
        1970 + year_start,
        when.as_secs() / 3_600 % 24,
        when.as_secs() / 60 % 60,
        when.as_secs() % 60
    )
}

/// The files of a site, with their content.
///
/// The names are the ones the panel knows about, so the editor and the
/// server agree on them.
pub const SITE_FILES: [&str; 3] = ["index.html", "style.css", "app.js"];

/// The name of the file that holds the page.
pub const PAGE: &str = "index.html";

/// The name of the file that holds the style.
pub const STYLE: &str = "style.css";

/// The name of the file that holds the script.
pub const SCRIPT: &str = "app.js";
