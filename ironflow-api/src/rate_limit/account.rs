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
    /// trimmed and lowercased, per route: a sign-up flood does not exhaust
    /// the sign-in bucket of the same email. Meant for the credential routes.
    ByEmail,
}

#[derive(Deserialize)]
struct TargetAccount {
    email: String,
}

/// The account bucket a credential body targets on `route`.
///
/// `None` for a body that is not JSON or has no usable `email`: the handler
/// rejects it on its own, and the IP bucket has already counted it.
pub(super) fn account_key(route: &str, body: &[u8]) -> Option<RateLimitKey> {
    let TargetAccount { email } = from_slice(body).ok()?;
    let email = email.trim().to_lowercase();
    let usable = !email.is_empty() && email.len() <= MAX_EMAIL_LEN;
    usable.then(|| RateLimitKey::Account {
        route: route.to_string(),
        email,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign_in(email: &str) -> Option<RateLimitKey> {
        Some(RateLimitKey::Account {
            route: "/sign-in".to_string(),
            email: email.to_string(),
        })
    }

    #[test]
    fn email_is_trimmed_and_lowercased() {
        assert_eq!(
            account_key(
                "/sign-in",
                br#"{"email":"  Alice@IronFlow.dev ","password":"x"}"#
            ),
            sign_in("alice@ironflow.dev")
        );
    }

    #[test]
    fn body_without_email_has_no_account() {
        assert_eq!(account_key("/sign-in", br#"{"password":"x"}"#), None);
        assert_eq!(account_key("/sign-in", br#"{"email":"   "}"#), None);
        assert_eq!(account_key("/sign-in", br#"{"email":42}"#), None);
    }

    #[test]
    fn non_json_body_has_no_account() {
        assert_eq!(account_key("/sign-in", b"email=alice@ironflow.dev"), None);
        assert_eq!(account_key("/sign-in", b""), None);
    }

    #[test]
    fn oversized_email_has_no_account() {
        let long = format!(r#"{{"email":"{}@x.dev"}}"#, "a".repeat(MAX_EMAIL_LEN));
        assert_eq!(account_key("/sign-in", long.as_bytes()), None);
    }

    #[test]
    fn unicode_email_is_lowercased() {
        assert_eq!(
            account_key("/sign-in", r#"{"email":"ÉLODIE@x.dev"}"#.as_bytes()),
            sign_in("élodie@x.dev")
        );
    }

    #[test]
    fn routes_get_distinct_account_keys() {
        let body = br#"{"email":"alice@ironflow.dev"}"#;
        let sign_up = account_key("/sign-up", body);
        assert_eq!(account_key("/sign-in", body), sign_in("alice@ironflow.dev"));
        assert_eq!(
            sign_up,
            Some(RateLimitKey::Account {
                route: "/sign-up".to_string(),
                email: "alice@ironflow.dev".to_string(),
            })
        );
        assert_ne!(sign_up, account_key("/sign-in", body));
    }
}
