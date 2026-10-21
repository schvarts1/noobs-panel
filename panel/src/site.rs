//! The files of the site of an instance.
//!
//! An instance of the site preset, or of the hello world preset, serves the
//! files that are in its directory. The user of the instance can change
//! them, so the site is written in the browser.

use std::path::{Path, PathBuf};

/// The name of the file that holds the page.
pub const PAGE: &str = "index.html";

/// The name of the file that holds the style.
pub const STYLE: &str = "style.css";

/// The name of the file that holds the script.
pub const SCRIPT: &str = "app.js";

/// The files of a site, in the order they are shown in the editor.
pub const FILES: [&str; 3] = [PAGE, STYLE, SCRIPT];

/// The largest file that can be saved.
pub const MAX_FILE_BYTES: usize = 256 * 1024;

/// The directory of an instance.
pub fn instance_dir(data_dir: &Path, instance_id: i64) -> PathBuf {
    data_dir.join(format!("instance-{instance_id}"))
}

/// The path of one file of the site.
pub fn file_path(data_dir: &Path, instance_id: i64, name: &str) -> Option<PathBuf> {
    if !FILES.contains(&name) {
        return None;
    }
    Some(instance_dir(data_dir, instance_id).join(name))
}

/// The page that is put in a new site.
pub const DEFAULT_PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>My site</title>
  <link rel="stylesheet" href="style.css">
</head>
<body>
  <main>
    <h1>Hello, this is my site</h1>
    <p>Change the html, the style and the script of your site in the editor.</p>
    <button id="hello">Say hello</button>
    <p id="output"></p>
  </main>
  <script src="app.js"></script>
</body>
</html>
"#;

/// The style that is put in a new site.
pub const DEFAULT_STYLE: &str = r#"body {
  margin: 0;
  font-family: system-ui, sans-serif;
  background: #101418;
  color: #e8edf2;
}

main {
  max-width: 40rem;
  margin: 4rem auto;
  padding: 0 1.5rem;
}

h1 {
  font-size: 2rem;
}

button {
  font: inherit;
  padding: .5rem 1rem;
  border: 0;
  border-radius: .4rem;
  background: #4a9de0;
  color: #06121c;
  cursor: pointer;
}
"#;

/// The script that is put in a new site.
pub const DEFAULT_SCRIPT: &str = r#"const button = document.getElementById('hello');
const output = document.getElementById('output');

if (button) {
  button.addEventListener('click', () => {
    output.textContent = `Hello from the browser, at ${new Date().toLocaleTimeString()}.`;
  });
}
"#;

/// The content a new site starts with.
pub fn defaults(name: &str) -> &'static str {
    match name {
        PAGE => DEFAULT_PAGE,
        STYLE => DEFAULT_STYLE,
        SCRIPT => DEFAULT_SCRIPT,
        _ => "",
    }
}

/// Put the files of a new site in the directory of the instance.
///
/// Nothing is written when the files are already there, so the work of the
/// user is not lost when the instance is stopped and started again.
pub fn seed(data_dir: &Path, instance_id: i64) -> std::io::Result<()> {
    let directory = instance_dir(data_dir, instance_id);
    std::fs::create_dir_all(&directory)?;

    for name in FILES {
        let path = directory.join(name);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, defaults(name))?;
    }

    Ok(())
}

/// The content of the files of a site.
///
/// A file that is not there yet comes back with the default content, so the
/// editor always has something to show.
pub fn read_all(data_dir: &Path, instance_id: i64) -> Vec<(String, String)> {
    let directory = instance_dir(data_dir, instance_id);
    FILES
        .into_iter()
        .map(|name| {
            let path = directory.join(name);
            let content =
                std::fs::read_to_string(&path).unwrap_or_else(|_| defaults(name).to_string());
            (name.to_string(), content)
        })
        .collect()
}

/// Save one file of a site.
///
/// The name of the file has to be one of the known ones, and the content may
/// not be too large.
pub fn write(
    data_dir: &Path,
    instance_id: i64,
    name: &str,
    content: &str,
) -> std::result::Result<(), String> {
    if !FILES.contains(&name) {
        return Err("That file is not part of the site.".into());
    }
    if content.len() > MAX_FILE_BYTES {
        return Err("That file is too large.".into());
    }

    let directory = instance_dir(data_dir, instance_id);
    std::fs::create_dir_all(&directory)
        .map_err(|err| format!("Cannot create the directory: {err}"))?;
    let path = directory.join(name);
    std::fs::write(&path, content).map_err(|err| format!("Cannot write the file: {err}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_not_empty() {
        for name in FILES {
            assert!(!defaults(name).is_empty(), "{name} has no default");
        }
    }

    #[test]
    fn the_page_links_the_style_and_the_script() {
        assert!(DEFAULT_PAGE.contains("style.css"));
        assert!(DEFAULT_PAGE.contains("app.js"));
    }

    #[test]
    fn only_known_files_can_be_written() {
        let directory = std::env::temp_dir().join("noobs-site-test");
        let _ = std::fs::remove_dir_all(&directory);

        assert!(write(&directory, 1, "evil.sh", "rm -rf /").is_err());
        assert!(write(&directory, 1, "index.html", "<h1>hi</h1>").is_ok());

        let path = directory.join("instance-1").join("index.html");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "<h1>hi</h1>");
    }

    #[test]
    fn a_file_that_is_too_large_is_refused() {
        let directory = std::env::temp_dir().join("noobs-site-test-big");
        let _ = std::fs::remove_dir_all(&directory);

        let content = "a".repeat(MAX_FILE_BYTES + 1);
        assert!(write(&directory, 1, "index.html", &content).is_err());

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn seeding_writes_the_files_of_the_site() {
        let directory = std::env::temp_dir().join("noobs-site-test-seed");
        let _ = std::fs::remove_dir_all(&directory);

        seed(&directory, 7).unwrap();
        for name in FILES {
            assert!(instance_dir(&directory, 7).join(name).is_file());
        }

        // The work of the user is not thrown away.
        write(&directory, 7, PAGE, "mine").unwrap();
        seed(&directory, 7).unwrap();
        let content = read_all(&directory, 7);
        let page = content.iter().find(|(name, _)| name == PAGE).unwrap();
        assert_eq!(page.1, "mine");

        let _ = std::fs::remove_dir_all(&directory);
    }
}
