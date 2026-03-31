//! Argon2id password hashing and verification.

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use crate::error::AuthError;

/// Hash a plaintext password using Argon2id.
///
/// # Errors
///
/// Returns [`AuthError::PasswordHash`] if hashing fails.
///
/// # Examples
///
/// ```
/// use ironflow_auth::password;
///
/// let hash = password::hash("hunter2").unwrap();
/// assert!(hash.starts_with("$argon2id$"));
/// ```
pub fn hash(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| AuthError::PasswordHash)?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against an Argon2id hash.
///
/// # Errors
///
/// Returns [`AuthError::PasswordHash`] if the hash is malformed.
///
/// # Examples
///
/// ```
/// use ironflow_auth::password;
///
/// let hash = password::hash("correct").unwrap();
/// assert!(password::verify("correct", &hash).unwrap());
/// assert!(!password::verify("wrong", &hash).unwrap());
/// ```
pub fn verify(password: &str, hash: &str) -> Result<bool, AuthError> {
    let parsed_hash = PasswordHash::new(hash).map_err(|_| AuthError::PasswordHash)?;
    let argon2 = Argon2::default();
    Ok(argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_produces_argon2id() {
        let h = hash("password123").unwrap();
        assert!(h.starts_with("$argon2id$"));
    }

    #[test]
    fn verify_correct_password() {
        let h = hash("mypassword").unwrap();
        assert!(verify("mypassword", &h).unwrap());
    }

    #[test]
    fn verify_wrong_password() {
        let h = hash("mypassword").unwrap();
        assert!(!verify("wrongpassword", &h).unwrap());
    }

    #[test]
    fn different_hashes_for_same_password() {
        let h1 = hash("same").unwrap();
        let h2 = hash("same").unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn malformed_hash_returns_error() {
        assert!(verify("pw", "not-a-valid-hash").is_err());
    }
}
