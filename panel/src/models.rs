//! Domain models shared across the panel.

use std::fmt;

use crate::presets::Preset;

/// Role of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A regular user. Can manage their own instances only.
    User,
    /// An administrator. Can manage every user and instance.
    Admin,
}

impl Role {
    /// Parse the role stored in the database.
    pub fn from_db(value: &str) -> Role {
        match value {
            "admin" => Role::Admin,
            _ => Role::User,
        }
    }

    /// The value stored in the database.
    pub fn as_db(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Admin => "admin",
        }
    }

    /// Whether this role may use the admin area.
    pub fn is_admin(&self) -> bool {
        matches!(self, Role::Admin)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_db())
    }
}

/// An account on the panel.
#[derive(Debug, Clone)]
pub struct User {
    /// Database id.
    pub id: i64,
    /// Login name, unique.
    pub username: String,
    /// Email address, unique.
    pub email: String,
    /// Role of the account.
    pub role: Role,
    /// Whether the account may sign in.
    pub is_active: bool,
    /// Creation timestamp, seconds since the epoch.
    pub created_at: i64,
    /// Total memory quota in megabytes across all owned instances.
    pub memory_limit_mb: i64,
    /// Total cpu time quota in seconds across all owned instances.
    pub cpu_limit_seconds: i64,
    /// Total disk quota in megabytes across all owned instances.
    pub disk_limit_mb: i64,
    /// Maximum number of instances the account may own.
    pub instance_limit: i64,
}

impl User {
    /// Whether the account holds admin rights.
    pub fn is_admin(&self) -> bool {
        self.role.is_admin()
    }
}

/// The lifecycle state of an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceState {
    /// The process is running.
    Running,
    /// The process was stopped on purpose.
    Stopped,
    /// The process exited unexpectedly.
    Crashed,
    /// The instance is starting up.
    Starting,
}

impl InstanceState {
    /// Parse the state stored in the database.
    pub fn from_db(value: &str) -> InstanceState {
        match value {
            "running" => InstanceState::Running,
            "crashed" => InstanceState::Crashed,
            "starting" => InstanceState::Starting,
            _ => InstanceState::Stopped,
        }
    }

    /// The value stored in the database.
    pub fn as_db(&self) -> &'static str {
        match self {
            InstanceState::Running => "running",
            InstanceState::Stopped => "stopped",
            InstanceState::Crashed => "crashed",
            InstanceState::Starting => "starting",
        }
    }

    /// A short human readable label.
    pub fn label(&self) -> &'static str {
        match self {
            InstanceState::Running => "Running",
            InstanceState::Stopped => "Stopped",
            InstanceState::Crashed => "Crashed",
            InstanceState::Starting => "Starting",
        }
    }
}

impl fmt::Display for InstanceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A web server instance managed by the panel.
#[derive(Debug, Clone)]
pub struct Instance {
    /// Database id.
    pub id: i64,
    /// Display name, unique.
    pub name: String,
    /// Id of the owning account.
    pub owner_id: i64,
    /// Login name of the owning account.
    pub owner_username: String,
    /// Port the instance listens on.
    pub port: u16,
    /// Current lifecycle state.
    pub state: InstanceState,
    /// Process id of the running server, if any.
    pub pid: Option<i32>,
    /// Memory limit in megabytes.
    pub memory_limit_mb: i64,
    /// Cpu time limit in seconds.
    pub cpu_limit_seconds: i64,
    /// Disk limit in megabytes.
    pub disk_limit_mb: i64,
    /// Creation timestamp, seconds since the epoch.
    pub created_at: i64,
    /// Most recent recorded exit status, if the instance has crashed.
    pub exit_code: Option<i32>,
    /// The kind of workload the instance runs.
    pub preset: Preset,
}

impl Instance {
    /// The url the instance can be reached at.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Whether the instance is currently serving requests.
    pub fn is_running(&self) -> bool {
        matches!(self.state, InstanceState::Running)
    }
}

/// A snapshot of the resources an instance is using right now.
#[derive(Debug, Clone, Default)]
pub struct Usage {
    /// Resident memory in kilobytes.
    pub memory_kb: u64,
    /// Cpu time consumed in seconds.
    pub cpu_seconds: u64,
    /// Disk space used by the instance in kilobytes.
    pub disk_kb: u64,
}

/// Resource limits that can be granted to an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Memory in megabytes.
    pub memory_mb: i64,
    /// Cpu time in seconds.
    pub cpu_seconds: i64,
    /// Disk space in megabytes.
    pub disk_mb: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_mb: 256,
            cpu_seconds: 60,
            disk_mb: 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_round_trips() {
        assert_eq!(Role::from_db("admin"), Role::Admin);
        assert_eq!(Role::from_db("user"), Role::User);
        assert_eq!(Role::from_db("nonsense"), Role::User);
        assert_eq!(Role::Admin.as_db(), "admin");
    }

    #[test]
    fn state_round_trips() {
        for state in [
            InstanceState::Running,
            InstanceState::Stopped,
            InstanceState::Crashed,
            InstanceState::Starting,
        ] {
            assert_eq!(InstanceState::from_db(state.as_db()), state);
        }
    }

    #[test]
    fn instance_url_uses_its_port() {
        let instance = Instance {
            id: 1,
            name: "web".into(),
            owner_id: 1,
            owner_username: "alice".into(),
            port: 8123,
            state: InstanceState::Running,
            pid: Some(42),
            memory_limit_mb: 128,
            cpu_limit_seconds: 30,
            disk_limit_mb: 512,
            created_at: 0,
            exit_code: None,
            preset: Preset::HelloWorld,
        };
        assert_eq!(instance.url(), "http://127.0.0.1:8123");
        assert!(instance.is_running());
    }
}
