//! Password hashing, session tokens and CSRF protection.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::Rng;

use crate::db::{Database, Result};

/// Name of the cookie carrying the session token.
pub const SESSION_COOKIE: &str = "noobs_session";

/// Name of the cookie carrying the csrf token of a form that is shown before
/// signing in.
pub const CSRF_COOKIE: &str = "noobs_csrf";

/// Generate a random token, hex encoded.
fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Generate a csrf token for a form.
pub fn generate_csrf_token() -> String {
    random_token(32)
}

/// Whether a string has the shape of a csrf token.
///
/// The check keeps a request with a nonsense cookie from reaching the
/// database.
pub fn is_csrf_token(token: &str) -> bool {
    token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit())
}

/// Hash a password with Argon2id.
pub fn hash_password(password: &str) -> Result<String> {
    let hash = Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|err| crate::db::DbError::Conflict(format!("cannot hash password: {err}")))?;
    Ok(hash.to_string())
}

/// Check a password against a stored Argon2 hash.
///
/// A malformed hash is treated as a failed verification rather than an
/// error, so a corrupted record can never be used to sign in.
pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// A freshly created session.
pub struct NewSession {
    /// Token stored in the cookie.
    pub token: String,
    /// Token embedded in forms to protect against cross site requests.
    pub csrf_token: String,
    /// When the session stops being valid, seconds since the epoch.
    pub expires_at: i64,
}

/// Create and store a new session for a user.
pub fn create_session(
    db: &Database,
    user_id: i64,
    now: i64,
    ttl_seconds: i64,
) -> Result<NewSession> {
    let token = random_token(32);
    let csrf_token = random_token(32);
    let expires_at = now + ttl_seconds;
    db.create_session(&token, user_id, &csrf_token, now, expires_at)?;
    Ok(NewSession {
        token,
        csrf_token,
        expires_at,
    })
}

/// Check whether a csrf token matches the one held by the session.
///
/// An empty token never matches, otherwise a form without a token would be
/// accepted as valid.
pub fn csrf_matches(expected: &str, provided: &str) -> bool {
    if expected.is_empty() || provided.is_empty() {
        return false;
    }
    // Both are hex strings of the same length, so a plain comparison of the
    // bytes is enough. The length is compared first to avoid leaking how
    // much of the token matched through timing.
    if expected.len() != provided.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in expected.bytes().zip(provided.bytes()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Validate a username.
pub fn validate_username(username: &str) -> std::result::Result<(), String> {
    if username.len() < 3 || username.len() > 32 {
        return Err("Username must be between 3 and 32 characters long.".into());
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(
            "Username may only contain letters, digits, dots, dashes and underscores.".into(),
        );
    }
    Ok(())
}

/// Validate an email address.
pub fn validate_email(email: &str) -> std::result::Result<(), String> {
    let trimmed = email.trim();
    if trimmed.len() < 3 || trimmed.len() > 254 {
        return Err("Please enter a valid email address.".into());
    }
    let mut parts = trimmed.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if parts.next().is_some() || local.is_empty() || domain.is_empty() {
        return Err("Please enter a valid email address.".into());
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return Err("Please enter a valid email address.".into());
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err("Please enter a valid email address.".into());
    }
    Ok(())
}

/// Validate a password.
pub fn validate_password(password: &str) -> std::result::Result<(), String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters long.".into());
    }
    if password.len() > 256 {
        return Err("Password must be at most 256 characters long.".into());
    }
    Ok(())
}

/// Validate an instance name.
pub fn validate_instance_name(name: &str) -> std::result::Result<(), String> {
    let trimmed = name.trim();
    if trimmed.len() < 3 || trimmed.len() > 32 {
        return Err("Instance name must be between 3 and 32 characters long.".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(
            "Instance name may only contain letters, digits, dashes and underscores.".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_verify_correctly() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong horse", &hash));
    }

    #[test]
    fn same_password_hashes_differently() {
        let first = hash_password("same").unwrap();
        let second = hash_password("same").unwrap();
        assert_ne!(first, second, "each hash must use a fresh salt");
    }

    #[test]
    fn malformed_hash_never_verifies() {
        assert!(!verify_password("anything", "not-a-hash"));
        assert!(!verify_password("anything", ""));
    }

    #[test]
    fn tokens_are_unique_and_hex() {
        let first = random_token(32);
        let second = random_token(32);
        assert_ne!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn csrf_requires_an_exact_match() {
        assert!(csrf_matches("abc123", "abc123"));
        assert!(!csrf_matches("abc123", "abc124"));
        assert!(!csrf_matches("abc123", "abc12"));
        assert!(!csrf_matches("abc123", ""));
    }

    #[test]
    fn validates_usernames() {
        assert!(validate_username("alice").is_ok());
        assert!(validate_username("a").is_err());
        assert!(validate_username("has space").is_err());
        assert!(validate_username("has/slash").is_err());
        assert!(validate_username(&"x".repeat(33)).is_err());
    }

    #[test]
    fn validates_emails() {
        assert!(validate_email("alice@example.com").is_ok());
        assert!(validate_email("no-at-sign").is_err());
        assert!(validate_email("two@@example.com").is_err());
        assert!(validate_email("a@b").is_err());
        assert!(validate_email("has space@example.com").is_err());
    }

    #[test]
    fn validates_passwords() {
        assert!(validate_password("longenough").is_ok());
        assert!(validate_password("short").is_err());
    }

    #[test]
    fn validates_instance_names() {
        assert!(validate_instance_name("my-web_1").is_ok());
        assert!(validate_instance_name("no").is_err());
        assert!(validate_instance_name("has space").is_err());
    }
}
