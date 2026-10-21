//! The janitor of the panel.
//!
//! Sessions and csrf tokens that have expired are of no use any more, and
//! the tables they are in grow until the panel is started again. The
//! janitor throws them away every few minutes, so the database stays
//! small.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::db::Database;

/// How often the janitor looks for the rows that have expired.
pub const INTERVAL: Duration = Duration::from_secs(300);

/// The thread that keeps the tables small.
pub struct Janitor {
    /// Told to stop when the panel stops.
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Janitor {
    /// Start the janitor with the usual interval.
    pub fn start(db: Arc<Database>) -> Janitor {
        Janitor::start_with_interval(db, INTERVAL)
    }

    /// Start the janitor with an interval of its own.
    pub fn start_with_interval(db: Arc<Database>, interval: Duration) -> Janitor {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();

        let handle = std::thread::spawn(move || {
            // The first turn is done right away, so the tables are small
            // from the start.
            prune(&db);

            // The wait is in short pieces, so the thread stops soon after
            // the panel says so.
            let pieces = interval.as_secs().max(1);
            for _ in 0..pieces {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }

            while !flag.load(Ordering::Relaxed) {
                prune(&db);
                for _ in 0..pieces {
                    if flag.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        });

        Janitor {
            stop,
            handle: Some(handle),
        }
    }

    /// Stop the janitor and wait for it.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Janitor {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Throw away the sessions and the csrf tokens that have expired.
pub fn prune(db: &Database) {
    let now = crate::app::now();
    let _ = db.prune_sessions(now);
    let _ = db.prune_csrf_tokens(now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Role;

    #[test]
    fn the_expired_rows_are_thrown_away() {
        let db = Database::open_in_memory().unwrap();
        let now = crate::app::now();

        // An account, so the sessions have something to point at.
        let user = db
            .create_user("alice", "alice@example.com", "hash", Role::User, now)
            .unwrap();

        // A session that is still good, and one that is not.
        db.create_session("good", user, "csrf", now, now + 3600)
            .unwrap();
        db.create_session("old", user, "csrf", now - 7200, now - 3600)
            .unwrap();

        // A csrf token that is still good, and one that is not.
        db.save_csrf_token("good", now, now + 3600).unwrap();
        db.save_csrf_token("old", now - 7200, now - 3600).unwrap();

        prune(&db);

        assert!(db.session_user("good", now).unwrap().is_some());
        assert!(db.session_user("old", now).unwrap().is_none());
        assert!(db.csrf_token_is_valid("good", now).unwrap());
        assert!(!db.csrf_token_is_valid("old", now).unwrap());
    }

    #[test]
    fn the_janitor_keeps_the_tables_small() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let now = crate::app::now();

        // An account, so the session has something to point at.
        let user = db
            .create_user("alice", "alice@example.com", "hash", Role::User, now)
            .unwrap();

        for number in 0..5 {
            db.create_session(
                &format!("old-{number}"),
                user,
                "csrf",
                now - 7200,
                now - 3600,
            )
            .unwrap();
        }

        let mut janitor = Janitor::start_with_interval(db.clone(), Duration::from_secs(1));

        // The janitor throws the old sessions away.
        let mut gone = false;
        for _ in 0..50 {
            if db.session_user("old-0", now).unwrap().is_none() {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(gone, "the janitor should have thrown the old sessions away");

        janitor.stop();
    }
}
