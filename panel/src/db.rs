//! SQLite storage for accounts, sessions and instances.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{Instance, InstanceState, Role, User};
use crate::presets::Preset;

/// Errors coming out of the storage layer.
#[derive(Debug)]
pub enum DbError {
    /// The underlying SQLite error.
    Sqlite(rusqlite::Error),
    /// A record that had to exist does not.
    NotFound(&'static str),
    /// The change would break a uniqueness rule.
    Conflict(String),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Sqlite(err) => write!(f, "database error: {err}"),
            DbError::NotFound(what) => write!(f, "{what} not found"),
            DbError::Conflict(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(err: rusqlite::Error) -> Self {
        DbError::Sqlite(err)
    }
}

/// Result alias for storage operations.
pub type Result<T> = std::result::Result<T, DbError>;

/// Handle to the panel database.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (and create if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Database> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| DbError::Conflict(format!("cannot create data directory: {err}")))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Database { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open an in-memory database, used by the tests.
    pub fn open_in_memory() -> Result<Database> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Database { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Create the schema if it is not there yet.
    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                username          TEXT    NOT NULL UNIQUE,
                email             TEXT    NOT NULL UNIQUE,
                password_hash     TEXT    NOT NULL,
                role              TEXT    NOT NULL DEFAULT 'user',
                is_active         INTEGER NOT NULL DEFAULT 1,
                created_at        INTEGER NOT NULL,
                memory_limit_mb   INTEGER NOT NULL DEFAULT 256,
                cpu_limit_percent INTEGER NOT NULL DEFAULT 200,
                disk_limit_mb     INTEGER NOT NULL DEFAULT 1024,
                instance_limit    INTEGER NOT NULL DEFAULT 5
            );

            CREATE TABLE IF NOT EXISTS sessions (
                token      TEXT    PRIMARY KEY,
                user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                csrf_token TEXT    NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS sessions_user ON sessions(user_id);

            CREATE TABLE IF NOT EXISTS csrf_tokens (
                token      TEXT    PRIMARY KEY,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS instances (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                name              TEXT    NOT NULL UNIQUE,
                owner_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                port              INTEGER NOT NULL UNIQUE,
                state             TEXT    NOT NULL DEFAULT 'stopped',
                pid               INTEGER,
                memory_limit_mb   INTEGER NOT NULL DEFAULT 256,
                cpu_limit_percent INTEGER NOT NULL DEFAULT 50,
                disk_limit_mb     INTEGER NOT NULL DEFAULT 1024,
                created_at        INTEGER NOT NULL,
                exit_code         INTEGER
            );

            CREATE INDEX IF NOT EXISTS instances_owner ON instances(owner_id);
            "#,
        )?;

        // The tables above are only made when they are not there yet, so a
        // table from an earlier version is brought up to date by hand.
        self.add_column_if_missing("instances", "preset", "TEXT NOT NULL DEFAULT 'hello-world'")?;
        self.add_column_if_missing("users", "cpu_limit_percent", "INTEGER NOT NULL DEFAULT 200")?;
        self.add_column_if_missing(
            "instances",
            "cpu_limit_percent",
            "INTEGER NOT NULL DEFAULT 50",
        )?;

        Ok(())
    }

    /// Add a column to a table, unless the table already has it.
    fn add_column_if_missing(&self, table: &str, column: &str, definition: &str) -> Result<()> {
        let found = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
            params![table, column],
            |row| row.get::<_, i64>(0),
        )?;
        if found == 0 {
            self.conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))?;
        }
        Ok(())
    }

    /// The number of registered accounts.
    pub fn user_count(&self) -> Result<i64> {
        let count = self
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get::<_, i64>(0))?;
        Ok(count)
    }

    /// Create an account. Returns the new id.
    pub fn create_user(
        &self,
        username: &str,
        email: &str,
        password_hash: &str,
        role: Role,
        created_at: i64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO users (username, email, password_hash, role, is_active, created_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)",
            params![username, email, password_hash, role.as_db(), created_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Look up an account by login name.
    pub fn user_by_username(&self, username: &str) -> Result<Option<User>> {
        let user = self
            .conn
            .query_row(
                "SELECT id, username, email, role, is_active, created_at,
                        memory_limit_mb, cpu_limit_percent, disk_limit_mb, instance_limit
                 FROM users WHERE username = ?1",
                params![username],
                map_user,
            )
            .optional()?;
        Ok(user)
    }

    /// Look up an account by id.
    pub fn user_by_id(&self, id: i64) -> Result<Option<User>> {
        let user = self
            .conn
            .query_row(
                "SELECT id, username, email, role, is_active, created_at,
                        memory_limit_mb, cpu_limit_percent, disk_limit_mb, instance_limit
                 FROM users WHERE id = ?1",
                params![id],
                map_user,
            )
            .optional()?;
        Ok(user)
    }

    /// Look up an account by email address.
    pub fn user_by_email(&self, email: &str) -> Result<Option<User>> {
        let user = self
            .conn
            .query_row(
                "SELECT id, username, email, role, is_active, created_at,
                        memory_limit_mb, cpu_limit_percent, disk_limit_mb, instance_limit
                 FROM users WHERE email = ?1",
                params![email],
                map_user,
            )
            .optional()?;
        Ok(user)
    }

    /// The stored password hash of an account, used to verify a sign in.
    pub fn password_hash(&self, username: &str) -> Result<Option<String>> {
        let hash = self
            .conn
            .query_row(
                "SELECT password_hash FROM users WHERE username = ?1",
                params![username],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(hash)
    }

    /// Every account, oldest first.
    pub fn all_users(&self) -> Result<Vec<User>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, username, email, role, is_active, created_at,
                    memory_limit_mb, cpu_limit_percent, disk_limit_mb, instance_limit
             FROM users ORDER BY id",
        )?;
        let users = stmt
            .query_map([], map_user)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(users)
    }

    /// Change the role of an account.
    pub fn set_role(&self, user_id: i64, role: Role) -> Result<()> {
        self.conn.execute(
            "UPDATE users SET role = ?1 WHERE id = ?2",
            params![role.as_db(), user_id],
        )?;
        Ok(())
    }

    /// Enable or disable an account.
    pub fn set_active(&self, user_id: i64, active: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE users SET is_active = ?1 WHERE id = ?2",
            params![active as i64, user_id],
        )?;
        Ok(())
    }

    /// Change the password of an account.
    pub fn set_password(&self, user_id: i64, password_hash: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![password_hash, user_id],
        )?;
        Ok(())
    }

    /// Update the resource limits granted to an account.
    pub fn set_limits(
        &self,
        user_id: i64,
        memory_mb: i64,
        cpu_percent: i64,
        disk_mb: i64,
        instance_limit: i64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE users
             SET memory_limit_mb = ?1, cpu_limit_percent = ?2, disk_limit_mb = ?3, instance_limit = ?4
             WHERE id = ?5",
            params![memory_mb, cpu_percent, disk_mb, instance_limit, user_id],
        )?;
        Ok(())
    }

    /// Delete an account and everything that belongs to it.
    pub fn delete_user(&self, user_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM users WHERE id = ?1", params![user_id])?;
        Ok(())
    }

    /// Store a new session.
    pub fn create_session(
        &self,
        token: &str,
        user_id: i64,
        csrf_token: &str,
        created_at: i64,
        expires_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (token, user_id, csrf_token, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![token, user_id, csrf_token, created_at, expires_at],
        )?;
        Ok(())
    }

    /// The user id and csrf token of a session, if it exists and is valid.
    pub fn session_user(&self, token: &str, now: i64) -> Result<Option<(i64, String)>> {
        let session = self
            .conn
            .query_row(
                "SELECT user_id, csrf_token FROM sessions WHERE token = ?1 AND expires_at > ?2",
                params![token, now],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        Ok(session)
    }

    /// Remove a session.
    pub fn delete_session(&self, token: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM sessions WHERE token = ?1", params![token])?;
        Ok(())
    }

    /// Remove every session that has expired.
    pub fn prune_sessions(&self, now: i64) -> Result<usize> {
        let removed = self
            .conn
            .execute("DELETE FROM sessions WHERE expires_at <= ?1", params![now])?;
        Ok(removed)
    }

    /// Remember a csrf token that was handed out for a form that is shown
    /// before signing in.
    pub fn save_csrf_token(&self, token: &str, created_at: i64, expires_at: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO csrf_tokens (token, created_at, expires_at)
             VALUES (?1, ?2, ?3)",
            params![token, created_at, expires_at],
        )?;
        Ok(())
    }

    /// Whether a csrf token is one the panel handed out and is still valid.
    ///
    /// The token is kept on the server, so a cookie that was put in the
    /// browser by somebody else is not enough to pass the check.
    pub fn csrf_token_is_valid(&self, token: &str, now: i64) -> Result<bool> {
        let found = self
            .conn
            .query_row(
                "SELECT 1 FROM csrf_tokens WHERE token = ?1 AND expires_at > ?2",
                params![token, now],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Remove every csrf token that has expired.
    pub fn prune_csrf_tokens(&self, now: i64) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM csrf_tokens WHERE expires_at <= ?1",
            params![now],
        )?;
        Ok(removed)
    }

    /// Create an instance record.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn create_instance(
        &self,
        name: &str,
        owner_id: i64,
        port: u16,
        memory_limit_mb: i64,
        cpu_limit_percent: i64,
        disk_limit_mb: i64,
        preset: &str,
        created_at: i64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO instances
                 (name, owner_id, port, state, memory_limit_mb, cpu_limit_percent,
                  disk_limit_mb, preset, created_at)
             VALUES (?1, ?2, ?3, 'stopped', ?4, ?5, ?6, ?7, ?8)",
            params![
                name,
                owner_id,
                port,
                memory_limit_mb,
                cpu_limit_percent,
                disk_limit_mb,
                preset,
                created_at
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Hand an instance to another account.
    pub fn set_instance_owner(&self, id: i64, owner_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE instances SET owner_id = ?1 WHERE id = ?2",
            params![owner_id, id],
        )?;
        Ok(())
    }

    /// Look up an instance by id.
    pub fn instance_by_id(&self, id: i64) -> Result<Option<Instance>> {
        let instance = self
            .conn
            .query_row(
                "SELECT i.id, i.name, i.owner_id, u.username, i.port, i.state, i.pid,
                        i.memory_limit_mb, i.cpu_limit_percent, i.disk_limit_mb,
                        i.created_at, i.exit_code, i.preset
                 FROM instances i JOIN users u ON u.id = i.owner_id
                 WHERE i.id = ?1",
                params![id],
                map_instance,
            )
            .optional()?;
        Ok(instance)
    }

    /// Every instance, optionally filtered by owner.
    pub fn list_instances(&self, owner_id: Option<i64>) -> Result<Vec<Instance>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.name, i.owner_id, u.username, i.port, i.state, i.pid,
                    i.memory_limit_mb, i.cpu_limit_percent, i.disk_limit_mb,
                    i.created_at, i.exit_code, i.preset
             FROM instances i JOIN users u ON u.id = i.owner_id
             ORDER BY i.id",
        )?;
        let rows = stmt.query_map([], map_instance)?;
        let mut instances = Vec::new();
        for instance in rows {
            let instance = instance?;
            if owner_id.is_none_or(|owner| owner == instance.owner_id) {
                instances.push(instance);
            }
        }
        Ok(instances)
    }

    /// Update the runtime state of an instance.
    pub fn set_instance_state(
        &self,
        id: i64,
        state: InstanceState,
        pid: Option<i32>,
        exit_code: Option<i32>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE instances SET state = ?1, pid = ?2, exit_code = ?3 WHERE id = ?4",
            params![state.as_db(), pid, exit_code, id],
        )?;
        Ok(())
    }

    /// Update the port of an instance.
    ///
    /// An instance moves to another port when the one it was given is busy,
    /// so the port it ended up on has to be recorded.
    pub fn set_instance_port(&self, id: i64, port: u16) -> Result<()> {
        self.conn.execute(
            "UPDATE instances SET port = ?1 WHERE id = ?2",
            params![port, id],
        )?;
        Ok(())
    }

    /// Update the resource limits of an instance.
    pub fn set_instance_limits(
        &self,
        id: i64,
        memory_limit_mb: i64,
        cpu_limit_percent: i64,
        disk_limit_mb: i64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE instances SET memory_limit_mb = ?1, cpu_limit_percent = ?2, disk_limit_mb = ?3
             WHERE id = ?4",
            params![memory_limit_mb, cpu_limit_percent, disk_limit_mb, id],
        )?;
        Ok(())
    }

    /// Delete an instance record.
    pub fn delete_instance(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM instances WHERE id = ?1", params![id])?;
        Ok(())
    }
}

/// Map a user row.
fn map_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        email: row.get(2)?,
        role: Role::from_db(&row.get::<_, String>(3)?),
        is_active: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
        memory_limit_mb: row.get(6)?,
        cpu_limit_percent: row.get(7)?,
        disk_limit_mb: row.get(8)?,
        instance_limit: row.get(9)?,
    })
}

/// Map an instance row.
fn map_instance(row: &rusqlite::Row<'_>) -> rusqlite::Result<Instance> {
    Ok(Instance {
        id: row.get(0)?,
        name: row.get(1)?,
        owner_id: row.get(2)?,
        owner_username: row.get(3)?,
        port: row.get(4)?,
        state: InstanceState::from_db(&row.get::<_, String>(5)?),
        pid: row.get(6)?,
        memory_limit_mb: row.get(7)?,
        cpu_limit_percent: row.get(8)?,
        disk_limit_mb: row.get(9)?,
        created_at: row.get(10)?,
        exit_code: row.get(11)?,
        preset: Preset::from_slug(&row.get::<_, String>(12)?).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Limits;

    fn db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn creates_and_reads_a_user() {
        let db = db();
        let id = db
            .create_user("alice", "alice@example.com", "hash", Role::User, 100)
            .unwrap();
        let user = db.user_by_id(id).unwrap().unwrap();
        assert_eq!(user.username, "alice");
        assert_eq!(user.role, Role::User);
        assert!(user.is_active);
    }

    #[test]
    fn rejects_duplicate_usernames() {
        let db = db();
        db.create_user("alice", "a@example.com", "h", Role::User, 1)
            .unwrap();
        let err = db
            .create_user("alice", "b@example.com", "h", Role::User, 2)
            .unwrap_err();
        assert!(matches!(err, DbError::Sqlite(_)));
    }

    #[test]
    fn stores_and_finds_sessions() {
        let db = db();
        let user_id = db
            .create_user("alice", "a@example.com", "h", Role::User, 1)
            .unwrap();
        db.create_session("tok", user_id, "csrf", 100, 200).unwrap();
        let (found_id, csrf) = db.session_user("tok", 150).unwrap().unwrap();
        assert_eq!(found_id, user_id);
        assert_eq!(csrf, "csrf");
        // Expired sessions are not returned.
        assert!(db.session_user("tok", 250).unwrap().is_none());
    }

    #[test]
    fn prunes_expired_sessions() {
        let db = db();
        let user_id = db
            .create_user("alice", "a@example.com", "h", Role::User, 1)
            .unwrap();
        db.create_session("old", user_id, "c", 0, 50).unwrap();
        db.create_session("new", user_id, "c", 100, 500).unwrap();
        assert_eq!(db.prune_sessions(100).unwrap(), 1);
        assert!(db.session_user("old", 100).unwrap().is_none());
        assert!(db.session_user("new", 100).unwrap().is_some());
    }

    #[test]
    fn lists_instances_by_owner() {
        let db = db();
        let alice = db
            .create_user("alice", "a@example.com", "h", Role::User, 1)
            .unwrap();
        let bob = db
            .create_user("bob", "b@example.com", "h", Role::User, 1)
            .unwrap();
        db.create_instance("a-web", alice, 8100, 128, 30, 512, "hello-world", 1)
            .unwrap();
        db.create_instance("b-web", bob, 8101, 128, 30, 512, "hello-world", 1)
            .unwrap();

        let all = db.list_instances(None).unwrap();
        assert_eq!(all.len(), 2);
        let alices = db.list_instances(Some(alice)).unwrap();
        assert_eq!(alices.len(), 1);
        assert_eq!(alices[0].owner_username, "alice");
    }

    #[test]
    fn updates_limits_and_roles() {
        let db = db();
        let id = db
            .create_user("alice", "a@example.com", "h", Role::User, 1)
            .unwrap();
        db.set_role(id, Role::Admin).unwrap();
        db.set_limits(id, 512, 120, 2048, 10).unwrap();
        db.set_active(id, false).unwrap();
        let user = db.user_by_id(id).unwrap().unwrap();
        assert!(user.is_admin());
        assert_eq!(user.memory_limit_mb, 512);
        assert_eq!(user.cpu_limit_percent, 120);
        assert_eq!(user.disk_limit_mb, 2048);
        assert_eq!(user.instance_limit, 10);
        assert!(!user.is_active);
    }

    #[test]
    fn deleting_a_user_removes_their_instances() {
        let db = db();
        let id = db
            .create_user("alice", "a@example.com", "h", Role::User, 1)
            .unwrap();
        db.create_instance("a-web", id, 8100, 128, 30, 512, "hello-world", 1)
            .unwrap();
        db.delete_user(id).unwrap();
        assert!(db.list_instances(None).unwrap().is_empty());
    }

    #[test]
    fn default_limits_are_applied_to_new_users() {
        let db = db();
        let id = db
            .create_user("alice", "a@example.com", "h", Role::User, 1)
            .unwrap();
        let user = db.user_by_id(id).unwrap().unwrap();
        let defaults = Limits::default();
        assert_eq!(user.memory_limit_mb, defaults.memory_mb);
        assert_eq!(user.cpu_limit_percent, defaults.cpu_percent);
        assert_eq!(user.disk_limit_mb, defaults.disk_mb);
    }
}
