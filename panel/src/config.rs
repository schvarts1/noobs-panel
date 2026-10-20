//! Panel configuration.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

/// Default port the panel listens on.
pub const DEFAULT_PORT: u16 = 8080;

/// Default port range instances are allocated from.
pub const DEFAULT_INSTANCE_PORT_RANGE: (u16, u16) = (8100, 8999);

/// How long a login session stays valid.
pub const SESSION_TTL_SECONDS: i64 = 60 * 60 * 24 * 7;

/// How long a freshly created account may be used to log in.
pub const PENDING_TTL_SECONDS: i64 = 60 * 60 * 24;

/// Runtime configuration for the panel.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the panel binds to.
    pub host: IpAddr,
    /// Port the panel tries to bind.
    pub port: u16,
    /// Path of the SQLite database file.
    pub database: PathBuf,
    /// Directory where instance working directories live.
    pub data_dir: PathBuf,
    /// Range of ports instances are allocated from.
    pub instance_ports: (u16, u16),
    /// Whether the first account to register becomes an admin.
    pub first_user_is_admin: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            database: default_data_dir().join("panel.db"),
            data_dir: default_data_dir().join("instances"),
            instance_ports: DEFAULT_INSTANCE_PORT_RANGE,
            first_user_is_admin: true,
        }
    }
}

/// The directory used to store panel state.
pub fn default_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NOOBS_PANEL_DATA") {
        return PathBuf::from(dir);
    }
    // Termux keeps the home directory inside the app sandbox, which is the
    // right place for state that should survive an app restart.
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".noobs-panel"),
        Err(_) => PathBuf::from(".noobs-panel"),
    }
}

impl Config {
    /// Read configuration from the environment, falling back to defaults.
    pub fn from_env() -> Config {
        let mut config = Config::default();

        if let Ok(host) = std::env::var("NOOBS_PANEL_HOST") {
            if let Ok(parsed) = host.parse::<IpAddr>() {
                config.host = parsed;
            }
        }
        if let Ok(port) = std::env::var("NOOBS_PANEL_PORT") {
            if let Ok(parsed) = port.parse::<u16>() {
                config.port = parsed;
            }
        }
        if let Ok(dir) = std::env::var("NOOBS_PANEL_DATA") {
            let dir = PathBuf::from(dir);
            config.database = dir.join("panel.db");
            config.data_dir = dir.join("instances");
        }
        if let Ok(range) = std::env::var("NOOBS_PANEL_INSTANCE_PORTS") {
            if let Some((start, end)) = parse_port_range(&range) {
                config.instance_ports = (start, end);
            }
        }
        if let Ok(value) = std::env::var("NOOBS_PANEL_FIRST_USER_ADMIN") {
            config.first_user_is_admin = !matches!(value.as_str(), "0" | "false" | "no");
        }

        config
    }
}

/// Parse a `start-end` port range.
fn parse_port_range(value: &str) -> Option<(u16, u16)> {
    let (start, end) = value.split_once('-')?;
    let start = start.trim().parse::<u16>().ok()?;
    let end = end.trim().parse::<u16>().ok()?;
    if start == 0 || end == 0 || start > end {
        return None;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_port_ranges() {
        assert_eq!(parse_port_range("8100-8999"), Some((8100, 8999)));
        assert_eq!(parse_port_range(" 8000 - 8100 "), Some((8000, 8100)));
        assert_eq!(parse_port_range("9000-8000"), None);
        assert_eq!(parse_port_range("0-100"), None);
        assert_eq!(parse_port_range("nope"), None);
    }

    #[test]
    fn defaults_are_sane() {
        let config = Config::default();
        assert_eq!(config.port, 8080);
        assert_eq!(config.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(config.instance_ports.0 < config.instance_ports.1);
    }
}
