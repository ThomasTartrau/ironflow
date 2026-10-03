//! Per-account rate limiting for the credential routes.
//!
//! The IP bucket alone lets an attacker aim at one account from many
//! addresses. The account bucket counts every attempt on an email, whatever
//! address it comes from.

use serde::Deserialize;
use serde_json::from_slice;

use super::RateLimitKey;

/// Largest body read to find the targeted account. Credential bodies are a
/// few hundred bytes.
pub(super) const ACCOUNT_BODY_LIMIT: usize = 64 * 1024;

/// Longest email kept as an account key (RFC 5321 path limit). A longer one
/// matches no account, so it is not worth a counter.
const MAX_EMAIL_LEN: usize = 320;

/// Whether the limiter also counts requests against the account they target.
///
/// # Examples
///
/// ```
/// use ironflow_api::rate_limit::AccountLimit;
///
/// assert_eq!(AccountLimit::default(), AccountLimit::Off);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AccountLimit {
    /// Only the caller (API key, user or IP) is counted.
    #[default]
    Off,
    /// A JSON body with an `email` field is also counted against that email,
    /// trimmed and lowercased. Meant for the credential routes.
    ByEmail,
}

#[derive(Deserialize)]
struct TargetAccount {
    email: String,
}

/// The account bucket a credential body targets.
///
/// `None` for a body that is not JSON or has no usable `email`: the handler
/// rejects it on its own, and the IP bucket has already counted it.
pub(super) fn account_key(body: &[u8]) -> Option<RateLimitKey> {
    let TargetAccount { email } = from_slice(body).ok()?;
    let email = email.trim().to_lowercase();
    let usable = !email.is_empty() && email.len() <= MAX_EMAIL_LEN;
    usable.then_some(RateLimitKey::Account(email))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_trimmed_and_lowercased() {
        assert_eq!(
            account_key(br#"{"email":"  Alice@IronFlow.dev ","password":"x"}"#),
            Some(RateLimitKey::Account("alice@ironflow.dev".to_string()))
        );
    }

    #[test]
    fn body_without_email_has_no_account() {
        assert_eq!(account_key(br#"{"password":"x"}"#), None);
        assert_eq!(account_key(br#"{"email":"   "}"#), None);
        assert_eq!(account_key(br#"{"email":42}"#), None);
    }

    #[test]
    fn non_json_body_has_no_account() {
        assert_eq!(account_key(b"email=alice@ironflow.dev"), None);
        assert_eq!(account_key(b""), None);
    }

    #[test]
    fn oversized_email_has_no_account() {
        let long = format!(r#"{{"email":"{}@x.dev"}}"#, "a".repeat(MAX_EMAIL_LEN));
        assert_eq!(account_key(long.as_bytes()), None);
    }

    #[test]
    fn unicode_email_is_lowercased() {
        assert_eq!(
            account_key(r#"{"email":"ÉLODIE@x.dev"}"#.as_bytes()),
            Some(RateLimitKey::Account("élodie@x.dev".to_string()))
        );
    }
}
