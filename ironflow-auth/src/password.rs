//! Argon2id password hashing, verification and strength policy.

use std::collections::HashSet;
use std::sync::LazyLock;

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use thiserror::Error;

use crate::error::AuthError;

/// Minimum password length accepted by [`check_strength`], in characters.
pub const MIN_PASSWORD_LEN: usize = 12;

/// Maximum password length accepted by [`check_strength`], in characters.
pub const MAX_PASSWORD_LEN: usize = 128;

/// Fewest distinct characters a password may contain.
const MIN_DISTINCT_CHARS: usize = 5;

/// Shortest email or username fragment searched for inside a password.
/// Shorter fragments ("al", "x") would reject unrelated passwords.
const MIN_PERSONAL_FRAGMENT_LEN: usize = 3;

/// The SecLists `10k-most-common` list (MIT, see `common_passwords.LICENSE`).
static COMMON_PASSWORDS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    include_str!("common_passwords.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_lowercase)
        .collect()
});

/// Why a password was refused by [`check_strength`].
///
/// The messages are written for the person choosing the password.
///
/// # Examples
///
/// ```
/// use ironflow_auth::password::{PasswordPolicyError, check_strength};
///
/// let err = check_strength("short", &[]).unwrap_err();
/// assert_eq!(err, PasswordPolicyError::TooShort { min: 12 });
/// assert_eq!(err.to_string(), "password must be at least 12 characters");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PasswordPolicyError {
    /// Fewer than [`MIN_PASSWORD_LEN`] characters.
    #[error("password must be at least {min} characters")]
    TooShort {
        /// The minimum length, in characters.
        min: usize,
    },
    /// More than [`MAX_PASSWORD_LEN`] characters.
    #[error("password must be at most {max} characters")]
    TooLong {
        /// The maximum length, in characters.
        max: usize,
    },
    /// The password contains the email address, its local part, or the username.
    #[error("password must not contain your email address or username")]
    ContainsPersonalInfo,
    /// The password, or the word left once leading and trailing digits and
    /// symbols are removed, is a commonly used password.
    #[error("password is too common, choose a less predictable one")]
    TooCommon,
    /// The password repeats a short pattern or uses too few distinct characters.
    #[error("password is too repetitive, use more varied characters")]
    TooRepetitive,
}

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

/// Check a new password against the strength policy.
///
/// `personal` holds what the account is known by (email address, username).
/// The checks, in order:
///
/// 1. between [`MIN_PASSWORD_LEN`] and [`MAX_PASSWORD_LEN`] characters
///    (Unicode scalar values, not bytes);
/// 2. does not contain, case-insensitively, an entry of `personal`, nor the
///    local part of an email address in it (fragments under 3 characters
///    are ignored);
/// 3. neither the password nor its core (leading and trailing digits and
///    symbols removed, so `Password2024!!` gives `password`) is a common
///    password;
/// 4. not a short pattern repeated (`passwordpassword`), and at least five
///    distinct characters.
///
/// Call it wherever a password is chosen. Sign-in never calls it: existing
/// passwords keep working.
///
/// # Errors
///
/// Returns the first [`PasswordPolicyError`] the password violates.
///
/// # Examples
///
/// ```
/// use ironflow_auth::password::{PasswordPolicyError, check_strength};
///
/// let personal = ["alice@ironflow.dev", "alice"];
/// assert!(check_strength("correct horse battery staple", &personal).is_ok());
/// assert_eq!(
///     check_strength("alice@ironflow.dev", &personal),
///     Err(PasswordPolicyError::ContainsPersonalInfo),
/// );
/// assert_eq!(
///     check_strength("Password2024!!", &personal),
///     Err(PasswordPolicyError::TooCommon),
/// );
/// ```
pub fn check_strength(password: &str, personal: &[&str]) -> Result<(), PasswordPolicyError> {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(PasswordPolicyError::TooShort {
            min: MIN_PASSWORD_LEN,
        });
    }
    if len > MAX_PASSWORD_LEN {
        return Err(PasswordPolicyError::TooLong {
            max: MAX_PASSWORD_LEN,
        });
    }

    let lowered = password.to_lowercase();

    if personal_fragments(personal).any(|fragment| lowered.contains(&fragment)) {
        return Err(PasswordPolicyError::ContainsPersonalInfo);
    }

    let core = lowered.trim_matches(|c: char| !c.is_alphabetic());
    if COMMON_PASSWORDS.contains(&lowered) || COMMON_PASSWORDS.contains(core) {
        return Err(PasswordPolicyError::TooCommon);
    }

    let distinct: HashSet<char> = lowered.chars().collect();
    if distinct.len() < MIN_DISTINCT_CHARS || is_repeated_pattern(&lowered) {
        return Err(PasswordPolicyError::TooRepetitive);
    }

    Ok(())
}

/// Lowercased fragments of `personal` a password must not contain: each
/// entry, plus the local part of the ones that are email addresses.
fn personal_fragments<'a>(personal: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
    personal
        .iter()
        .flat_map(|entry| {
            let entry = entry.trim();
            let local = entry.split_once('@').map(|(local, _)| local);
            [Some(entry), local]
        })
        .flatten()
        .filter(|fragment| fragment.chars().count() >= MIN_PERSONAL_FRAGMENT_LEN)
        .map(str::to_lowercase)
}

/// Whether `s` is a shorter chunk repeated end to end (`abcabc`).
fn is_repeated_pattern(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    (1..=chars.len() / 2).any(|period| {
        chars.len().is_multiple_of(period)
            && chars.chunks(period).all(|chunk| chunk == &chars[..period])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERSONAL: [&str; 2] = ["alice@ironflow.dev", "alice"];

    #[test]
    fn strong_password_is_accepted() {
        assert_eq!(
            check_strength("correct horse battery staple", &PERSONAL),
            Ok(())
        );
        assert_eq!(check_strength("Tq9!vR2#mZ7$", &PERSONAL), Ok(()));
    }

    #[test]
    fn eleven_characters_is_too_short() {
        assert_eq!(
            check_strength("Tq9!vR2#mZ7", &[]),
            Err(PasswordPolicyError::TooShort { min: 12 })
        );
    }

    #[test]
    fn empty_password_is_too_short() {
        assert_eq!(
            check_strength("", &[]),
            Err(PasswordPolicyError::TooShort { min: 12 })
        );
    }

    #[test]
    fn length_counts_characters_not_bytes() {
        // 11 characters, 22 bytes: still too short.
        assert_eq!(
            check_strength("éàçùêâîôûëï", &[]),
            Err(PasswordPolicyError::TooShort { min: 12 })
        );
        // 12 characters, all distinct.
        assert_eq!(check_strength("éàçùêâîôûëïü", &[]), Ok(()));
    }

    #[test]
    fn length_bounds_are_inclusive() {
        let max: String = "Tq9!vR2#mZ7$".chars().cycle().take(128).collect();
        let over: String = "Tq9!vR2#mZ7$".chars().cycle().take(129).collect();
        // 128 is not a multiple of 12, so the cycle is not an exact repetition.
        assert_eq!(check_strength(&max, &[]), Ok(()));
        assert_eq!(
            check_strength(&over, &[]),
            Err(PasswordPolicyError::TooLong { max: 128 })
        );
    }

    #[test]
    fn password_equal_to_email_is_refused() {
        assert_eq!(
            check_strength("alice@ironflow.dev", &PERSONAL),
            Err(PasswordPolicyError::ContainsPersonalInfo)
        );
    }

    #[test]
    fn password_containing_username_is_refused_case_insensitively() {
        assert_eq!(
            check_strength("my-ALICE-secret-42", &PERSONAL),
            Err(PasswordPolicyError::ContainsPersonalInfo)
        );
    }

    #[test]
    fn password_containing_email_local_part_is_refused() {
        assert_eq!(
            check_strength("xx-jdoe.ops-2024", &["jdoe.ops@corp.example"]),
            Err(PasswordPolicyError::ContainsPersonalInfo)
        );
    }

    #[test]
    fn short_personal_fragments_are_ignored() {
        assert_eq!(check_strength("Tq9!vR2#mZ7$al", &["al@x.io", "al"]), Ok(()));
    }

    #[test]
    fn common_password_is_refused() {
        // In the list and at least 12 characters long.
        assert_eq!(
            check_strength("Unbelievable", &[]),
            Err(PasswordPolicyError::TooCommon)
        );
    }

    #[test]
    fn common_word_padded_with_digits_and_symbols_is_refused() {
        assert_eq!(
            check_strength("Password2024!!", &[]),
            Err(PasswordPolicyError::TooCommon)
        );
        assert_eq!(
            check_strength("!!!Qwertyuiop1", &[]),
            Err(PasswordPolicyError::TooCommon)
        );
    }

    #[test]
    fn repeated_pattern_is_refused() {
        assert_eq!(
            check_strength("passwordpassword", &[]),
            Err(PasswordPolicyError::TooRepetitive)
        );
        assert_eq!(
            check_strength("Xk9#Xk9#Xk9#", &[]),
            Err(PasswordPolicyError::TooRepetitive)
        );
    }

    #[test]
    fn too_few_distinct_characters_is_refused() {
        assert_eq!(
            check_strength("aaaa1111bbbb", &[]),
            Err(PasswordPolicyError::TooRepetitive)
        );
    }

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
