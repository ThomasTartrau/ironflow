//! Opaque tokens bound to one run and one step, held in memory by the proxy.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use getrandom::fill as fill_random;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::credential::ProxyCredential;
use super::{AuthProxyError, MAX_TOKEN_LIFETIME, TOKEN_PREFIX};

/// Length of [`IssuedToken::short_id`].
const SHORT_ID_LEN: usize = 12;

/// What the worker asks the proxy to issue a token for.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{CredentialKind, ProxyCredential, TokenRequest};
///
/// let request = TokenRequest {
///     run_id: "run-1".to_string(),
///     step: "review".to_string(),
///     expires_at: 1_700_000_600,
///     credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()),
/// };
/// assert!(!format!("{request:?}").contains("sk-ant"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRequest {
    /// Run the token is bound to.
    pub run_id: String,
    /// Step the token is bound to.
    pub step: String,
    /// Expiry, in unix seconds.
    pub expires_at: u64,
    /// Real credential the proxy injects for this token.
    pub credential: ProxyCredential,
}

/// A freshly issued opaque token. Its [`Debug`] output never shows the token.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{IssuedToken, token_id};
///
/// let token = "ifap_0123".to_string();
/// let issued = IssuedToken { id: token_id(&token), token };
/// assert_eq!(issued.short_id().len(), 12);
/// assert!(!format!("{issued:?}").contains("ifap_0123"));
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct IssuedToken {
    /// SHA-256 hex digest of the token: what revocation and logs use.
    pub id: String,
    /// The opaque token handed to the pod.
    pub token: String,
}

impl IssuedToken {
    /// First characters of [`id`](Self::id), for logs.
    ///
    /// # Examples
    ///
    /// See [`IssuedToken`].
    pub fn short_id(&self) -> &str {
        self.id.get(..SHORT_ID_LEN).unwrap_or(&self.id)
    }
}

impl fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedToken")
            .field("id", &self.id)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// What a valid token resolves to.
///
/// # Examples
///
/// See [`AuthProxyRegistry::resolve`].
#[derive(Debug, Clone)]
pub struct Grant {
    /// Run the token is bound to.
    pub run_id: String,
    /// Step the token is bound to.
    pub step: String,
    /// Expiry, in unix seconds.
    pub expires_at: u64,
    /// Real credential to inject.
    pub credential: ProxyCredential,
    /// Token id ([`token_id`]).
    pub id: String,
}

/// Why a token was refused.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{AuthProxyRegistry, TokenRejection};
///
/// let registry = AuthProxyRegistry::default();
/// assert_eq!(registry.resolve("ifap_nope", 0).err(), Some(TokenRejection::Unknown));
/// ```
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenRejection {
    /// The token was never issued, or was revoked.
    #[error("unknown token")]
    Unknown,
    /// The token has expired.
    #[error("expired token")]
    Expired,
}

/// In-memory registry of the issued tokens, keyed by [`token_id`]. The token
/// itself is never stored. Cheap to clone: clones share the same registry.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{
///     AuthProxyRegistry, CredentialKind, ProxyCredential, TokenRequest,
/// };
///
/// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let registry = AuthProxyRegistry::default();
/// let request = TokenRequest {
///     run_id: "run-1".to_string(),
///     step: "review".to_string(),
///     expires_at: 200,
///     credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()),
/// };
/// let issued = registry.issue(request, 100)?;
/// assert_eq!(registry.len(), 1);
/// assert!(registry.revoke(&issued.id));
/// assert!(registry.is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Default)]
pub struct AuthProxyRegistry {
    grants: Arc<Mutex<HashMap<String, Grant>>>,
}

impl AuthProxyRegistry {
    fn grants(&self) -> MutexGuard<'_, HashMap<String, Grant>> {
        self.grants.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Issue a new opaque token for `req`, `now` being the current unix time.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::InvalidRequest`] when `expires_at` is not
    /// after `now` or more than [`MAX_TOKEN_LIFETIME`] away, or when the run,
    /// step or credential is empty, or when the credential holds a character
    /// outside printable ASCII; [`AuthProxyError::Random`] when the system
    /// random source fails.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyRegistry`].
    pub fn issue(&self, req: TokenRequest, now: u64) -> Result<IssuedToken, AuthProxyError> {
        if req.expires_at <= now {
            return Err(invalid("expires_at is in the past"));
        }
        if req.expires_at - now > MAX_TOKEN_LIFETIME.as_secs() {
            return Err(invalid("expires_at is more than 24 hours away"));
        }
        if req.run_id.is_empty() {
            return Err(invalid("run_id is empty"));
        }
        if req.step.is_empty() {
            return Err(invalid("step is empty"));
        }
        let credential = req.credential.expose();
        if credential.is_empty() {
            return Err(invalid("credential is empty"));
        }
        if !credential.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(invalid("credential holds characters a header cannot carry"));
        }

        let mut bytes = [0u8; 32];
        fill_random(&mut bytes).map_err(|e| AuthProxyError::Random(e.to_string()))?;
        let token = format!("{TOKEN_PREFIX}{}", hex(&bytes));
        let id = token_id(&token);

        let grant = Grant {
            run_id: req.run_id,
            step: req.step,
            expires_at: req.expires_at,
            credential: req.credential,
            id: id.clone(),
        };
        self.grants().insert(id.clone(), grant);
        Ok(IssuedToken { id, token })
    }

    /// Resolve a token presented by a pod. An expired grant is removed.
    ///
    /// # Errors
    ///
    /// Returns [`TokenRejection::Unknown`] for a token never issued or
    /// revoked, [`TokenRejection::Expired`] once `expires_at <= now`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::{
    ///     AuthProxyRegistry, CredentialKind, ProxyCredential, TokenRejection, TokenRequest,
    /// };
    ///
    /// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// let registry = AuthProxyRegistry::default();
    /// let request = TokenRequest {
    ///     run_id: "run-1".to_string(),
    ///     step: "review".to_string(),
    ///     expires_at: 200,
    ///     credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()),
    /// };
    /// let issued = registry.issue(request, 100)?;
    /// assert_eq!(registry.resolve(&issued.token, 150).map(|g| g.run_id), Ok("run-1".to_string()));
    /// assert_eq!(registry.resolve(&issued.token, 200).err(), Some(TokenRejection::Expired));
    /// # Ok(())
    /// # }
    /// ```
    pub fn resolve(&self, token: &str, now: u64) -> Result<Grant, TokenRejection> {
        let id = token_id(token);
        let mut grants = self.grants();
        let grant = grants.get(&id).ok_or(TokenRejection::Unknown)?;
        if grant.expires_at <= now {
            grants.remove(&id);
            return Err(TokenRejection::Expired);
        }
        Ok(grant.clone())
    }

    /// Revoke a token by id. Returns whether it existed.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyRegistry`].
    pub fn revoke(&self, id: &str) -> bool {
        self.grants().remove(id).is_some()
    }

    /// Revoke every token of a run. Returns how many were revoked.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::AuthProxyRegistry;
    ///
    /// assert_eq!(AuthProxyRegistry::default().revoke_run("run-1"), 0);
    /// ```
    pub fn revoke_run(&self, run_id: &str) -> usize {
        let mut grants = self.grants();
        let before = grants.len();
        grants.retain(|_, grant| grant.run_id != run_id);
        before - grants.len()
    }

    /// Drop every grant expired at `now`. Returns how many were dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::AuthProxyRegistry;
    ///
    /// assert_eq!(AuthProxyRegistry::default().purge_expired(1_700_000_000), 0);
    /// ```
    pub fn purge_expired(&self, now: u64) -> usize {
        let mut grants = self.grants();
        let before = grants.len();
        grants.retain(|_, grant| grant.expires_at > now);
        before - grants.len()
    }

    /// Number of live grants.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyRegistry`].
    pub fn len(&self) -> usize {
        self.grants().len()
    }

    /// Whether no grant is held.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyRegistry`].
    pub fn is_empty(&self) -> bool {
        self.grants().is_empty()
    }
}

fn invalid(message: &str) -> AuthProxyError {
    AuthProxyError::InvalidRequest(message.to_string())
}

/// Lowercase hexadecimal encoding.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| [DIGITS[usize::from(b >> 4)], DIGITS[usize::from(b & 0x0f)]])
        .map(char::from)
        .collect()
}

/// SHA-256 hex digest of a token: its id in the registry, in revocation calls
/// and (shortened) in logs.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::token_id;
///
/// assert_eq!(token_id("a").len(), 64);
/// assert_eq!(token_id("a"), token_id("a"));
/// ```
pub fn token_id(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Compare an admin key in constant time (over the SHA-256 digests, so the
/// lengths do not leak either).
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::admin_key_matches;
///
/// assert!(admin_key_matches("secret", "secret"));
/// assert!(!admin_key_matches("secret", "secreT"));
/// ```
pub fn admin_key_matches(expected: &str, presented: &str) -> bool {
    let expected = Sha256::digest(expected.as_bytes());
    let presented = Sha256::digest(presented.as_bytes());
    expected
        .iter()
        .zip(presented.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::super::CredentialKind;
    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn request(run_id: &str, expires_at: u64) -> TokenRequest {
        TokenRequest {
            run_id: run_id.to_string(),
            step: "review".to_string(),
            expires_at,
            credential: ProxyCredential::new(
                CredentialKind::OauthToken,
                "sk-ant-oat01-test".to_string(),
            ),
        }
    }

    fn invalid_message(result: Result<IssuedToken, AuthProxyError>) -> String {
        match result {
            Err(AuthProxyError::InvalidRequest(message)) => message,
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn issue_then_resolve_returns_grant() {
        let registry = AuthProxyRegistry::default();
        let issued = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        let grant = registry.resolve(&issued.token, NOW + 1).unwrap();
        assert_eq!(grant.run_id, "run-1");
        assert_eq!(grant.step, "review");
        assert_eq!(grant.expires_at, NOW + 600);
        assert_eq!(grant.id, issued.id);
        assert_eq!(grant.credential.expose(), "sk-ant-oat01-test");
        assert_eq!(grant.credential.kind(), CredentialKind::OauthToken);
    }

    #[test]
    fn resolve_unknown_token_is_rejected() {
        let registry = AuthProxyRegistry::default();
        registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        assert_eq!(
            registry.resolve("invalide", NOW).unwrap_err(),
            TokenRejection::Unknown
        );
    }

    #[test]
    fn resolve_expired_token_is_rejected_and_removed() {
        let registry = AuthProxyRegistry::default();
        let issued = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        assert_eq!(
            registry.resolve(&issued.token, NOW + 600).unwrap_err(),
            TokenRejection::Expired
        );
        assert!(registry.is_empty());
        assert_eq!(
            registry.resolve(&issued.token, NOW + 600).unwrap_err(),
            TokenRejection::Unknown
        );
    }

    #[test]
    fn revoke_makes_token_unknown() {
        let registry = AuthProxyRegistry::default();
        let issued = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        assert!(registry.revoke(&issued.id));
        assert!(!registry.revoke(&issued.id));
        assert_eq!(
            registry.resolve(&issued.token, NOW).unwrap_err(),
            TokenRejection::Unknown
        );
    }

    #[test]
    fn revoke_run_only_drops_that_run() {
        let registry = AuthProxyRegistry::default();
        let a1 = registry.issue(request("run-a", NOW + 600), NOW).unwrap();
        let a2 = registry.issue(request("run-a", NOW + 600), NOW).unwrap();
        let b = registry.issue(request("run-b", NOW + 600), NOW).unwrap();
        assert_eq!(registry.revoke_run("run-a"), 2);
        assert_eq!(registry.revoke_run("run-a"), 0);
        assert!(registry.resolve(&a1.token, NOW).is_err());
        assert!(registry.resolve(&a2.token, NOW).is_err());
        assert_eq!(registry.resolve(&b.token, NOW).unwrap().run_id, "run-b");
    }

    #[test]
    fn purge_expired_counts() {
        let registry = AuthProxyRegistry::default();
        registry.issue(request("run-1", NOW + 10), NOW).unwrap();
        registry.issue(request("run-1", NOW + 20), NOW).unwrap();
        let live = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        assert_eq!(registry.purge_expired(NOW + 20), 2);
        assert_eq!(registry.len(), 1);
        assert!(registry.resolve(&live.token, NOW + 20).is_ok());
        assert_eq!(registry.purge_expired(NOW + 20), 0);
    }

    #[test]
    fn issue_rejects_past_expiry() {
        let registry = AuthProxyRegistry::default();
        let message = invalid_message(registry.issue(request("run-1", NOW), NOW));
        assert_eq!(message, "expires_at is in the past");
        let message = invalid_message(registry.issue(request("run-1", NOW - 1), NOW));
        assert_eq!(message, "expires_at is in the past");
        assert!(registry.is_empty());
    }

    #[test]
    fn issue_rejects_lifetime_over_24h() {
        let registry = AuthProxyRegistry::default();
        let max = MAX_TOKEN_LIFETIME.as_secs();
        let message = invalid_message(registry.issue(request("run-1", NOW + max + 1), NOW));
        assert!(message.contains("24 hours"), "{message}");
        assert!(registry.issue(request("run-1", NOW + max), NOW).is_ok());
    }

    #[test]
    fn issue_rejects_empty_credential() {
        let registry = AuthProxyRegistry::default();
        let mut req = request("run-1", NOW + 600);
        req.credential = ProxyCredential::new(CredentialKind::ApiKey, String::new());
        assert_eq!(
            invalid_message(registry.issue(req, NOW)),
            "credential is empty"
        );

        let message = invalid_message(registry.issue(request("", NOW + 600), NOW));
        assert_eq!(message, "run_id is empty");

        let mut req = request("run-1", NOW + 600);
        req.step = String::new();
        assert_eq!(invalid_message(registry.issue(req, NOW)), "step is empty");

        let mut req = request("run-1", NOW + 600);
        req.credential = ProxyCredential::new(CredentialKind::OauthToken, "tok\nen".to_string());
        let message = invalid_message(registry.issue(req, NOW));
        assert!(message.contains("header cannot carry"), "{message}");
        assert!(registry.is_empty());
    }

    #[test]
    fn token_has_prefix_and_never_starts_with_sk_ant() {
        let registry = AuthProxyRegistry::default();
        let issued = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        assert!(issued.token.starts_with(TOKEN_PREFIX));
        assert!(!issued.token.starts_with("sk-ant"));
        let suffix = &issued.token[TOKEN_PREFIX.len()..];
        assert_eq!(suffix.len(), 64);
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
        assert_eq!(issued.id, token_id(&issued.token));
        assert_eq!(issued.short_id(), &issued.id[..12]);
    }

    #[test]
    fn two_tokens_differ() {
        let registry = AuthProxyRegistry::default();
        let a = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        let b = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        assert_ne!(a.token, b.token);
        assert_ne!(a.id, b.id);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn registry_never_stores_the_token() {
        let registry = AuthProxyRegistry::default();
        let issued = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        let grants = registry.grants();
        let keys: Vec<&String> = grants.keys().collect();
        assert_eq!(keys, vec![&token_id(&issued.token)]);
        assert_ne!(keys[0], &issued.token);
        let stored = format!("{:?}", grants.values().collect::<Vec<_>>());
        assert!(!stored.contains(&issued.token), "{stored}");
        assert!(!stored.contains("sk-ant-oat01-test"), "{stored}");
    }

    #[test]
    fn issued_token_debug_redacts_token() {
        let registry = AuthProxyRegistry::default();
        let issued = registry.issue(request("run-1", NOW + 600), NOW).unwrap();
        let debug = format!("{issued:?}");
        assert!(!debug.contains(&issued.token), "{debug}");
        assert!(debug.contains(&issued.id), "{debug}");
    }

    #[test]
    fn admin_key_matches_true_false_and_different_lengths() {
        let key = "0123456789abcdef0123456789abcdef";
        assert!(admin_key_matches(key, key));
        assert!(!admin_key_matches(key, "0123456789abcdef0123456789abcdeF"));
        assert!(!admin_key_matches(key, "0123"));
        assert!(!admin_key_matches(key, ""));
        assert!(!admin_key_matches("", key));
    }
}
