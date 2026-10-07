//! Opaque tokens bound to one run and one step, held by the proxy in a
//! [`GrantBackend`].

use std::fmt;
use std::sync::Arc;

use getrandom::fill as fill_random;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::backend::{GrantBackend, MemoryGrantBackend};
use super::secret::GrantCredential;
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
///     credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()).into(),
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
    /// Real credential the proxy injects for this token: the Claude
    /// credential, or a proxied secret.
    pub credential: GrantCredential,
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
    pub credential: GrantCredential,
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
/// # async fn example() {
/// let registry = AuthProxyRegistry::default();
/// assert_eq!(registry.resolve("ifap_nope", 0).await.err(), Some(TokenRejection::Unknown));
/// # }
/// ```
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenRejection {
    /// The token was never issued, or its revocation is no longer
    /// remembered (expired since, or lost with an in-memory registry).
    #[error("unknown token")]
    Unknown,
    /// The token was revoked.
    #[error("revoked token")]
    Revoked,
    /// The token has expired.
    #[error("expired token")]
    Expired,
    /// The [`GrantBackend`] could not answer: the token may well be valid.
    /// The message never carries a token or credential value.
    #[error("token registry unavailable: {0}")]
    Unavailable(String),
}

/// Registry of the issued tokens, keyed by [`token_id`]. The token itself is
/// never stored. Grants live in a [`GrantBackend`]: in memory by default
/// ([`MemoryGrantBackend`]), or shared between replicas with
/// [`AuthProxyRegistry::with_backend`]. Cheap to clone: clones share the same
/// backend.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{
///     AuthProxyRegistry, CredentialKind, ProxyCredential, TokenRequest,
/// };
///
/// # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let registry = AuthProxyRegistry::default();
/// let request = TokenRequest {
///     run_id: "run-1".to_string(),
///     step: "review".to_string(),
///     expires_at: 200,
///     credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()).into(),
/// };
/// let issued = registry.issue(request, 100).await?;
/// assert_eq!(registry.len().await?, 1);
/// assert!(registry.revoke(&issued.id).await?);
/// assert!(registry.is_empty().await?);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AuthProxyRegistry {
    backend: Arc<dyn GrantBackend>,
}

impl Default for AuthProxyRegistry {
    fn default() -> Self {
        Self::with_backend(Arc::new(MemoryGrantBackend::default()))
    }
}

impl fmt::Debug for AuthProxyRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthProxyRegistry").finish_non_exhaustive()
    }
}

impl AuthProxyRegistry {
    /// A registry keeping its grants in `backend`. Registries built over the
    /// same backend (or over backends sharing the same storage, such as one
    /// database) see the same tokens.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use ironflow_core::auth_proxy::{
    ///     AuthProxyRegistry, CredentialKind, MemoryGrantBackend, ProxyCredential, TokenRequest,
    /// };
    ///
    /// # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// let backend = Arc::new(MemoryGrantBackend::default());
    /// let a = AuthProxyRegistry::with_backend(backend.clone());
    /// let b = AuthProxyRegistry::with_backend(backend);
    /// let request = TokenRequest {
    ///     run_id: "run-1".to_string(),
    ///     step: "review".to_string(),
    ///     expires_at: 200,
    ///     credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()).into(),
    /// };
    /// let issued = a.issue(request, 100).await?;
    /// assert!(b.resolve(&issued.token, 150).await.is_ok());
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_backend(backend: Arc<dyn GrantBackend>) -> Self {
        Self { backend }
    }

    /// Issue a new opaque token for `req`, `now` being the current unix time.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::InvalidRequest`] when `expires_at` is not
    /// after `now` or more than [`MAX_TOKEN_LIFETIME`] away, or when the run,
    /// step or credential is empty, or when the credential holds a character
    /// outside printable ASCII, or when a proxied secret fails
    /// [`SecretCredential::validate`](super::SecretCredential::validate);
    /// [`AuthProxyError::Random`] when the system
    /// random source fails; [`AuthProxyError::Backend`] when the backend
    /// cannot store the grant.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyRegistry`].
    pub async fn issue(&self, req: TokenRequest, now: u64) -> Result<IssuedToken, AuthProxyError> {
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
        match &req.credential {
            GrantCredential::Claude(claude) => {
                let credential = claude.expose();
                if credential.is_empty() {
                    return Err(invalid("credential is empty"));
                }
                if !credential.bytes().all(|b| b.is_ascii_graphic()) {
                    return Err(invalid("credential holds characters a header cannot carry"));
                }
            }
            GrantCredential::Secret(secret) => secret.validate()?,
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
        self.backend.insert(grant).await?;
        Ok(IssuedToken { id, token })
    }

    /// Resolve a token presented by a pod. An expired grant is removed.
    ///
    /// # Errors
    ///
    /// Returns [`TokenRejection::Unknown`] for a token never issued (or whose
    /// revocation has been purged), [`TokenRejection::Revoked`] for a revoked
    /// token, [`TokenRejection::Expired`] once `expires_at <= now`,
    /// [`TokenRejection::Unavailable`] when the backend fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::{
    ///     AuthProxyRegistry, CredentialKind, ProxyCredential, TokenRejection, TokenRequest,
    /// };
    ///
    /// # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// let registry = AuthProxyRegistry::default();
    /// let request = TokenRequest {
    ///     run_id: "run-1".to_string(),
    ///     step: "review".to_string(),
    ///     expires_at: 200,
    ///     credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()).into(),
    /// };
    /// let issued = registry.issue(request, 100).await?;
    /// assert_eq!(
    ///     registry.resolve(&issued.token, 150).await.map(|g| g.run_id),
    ///     Ok("run-1".to_string())
    /// );
    /// assert_eq!(
    ///     registry.resolve(&issued.token, 200).await.err(),
    ///     Some(TokenRejection::Expired)
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resolve(&self, token: &str, now: u64) -> Result<Grant, TokenRejection> {
        let id = token_id(token);
        let unavailable = |e: AuthProxyError| TokenRejection::Unavailable(e.to_string());
        let Some(grant) = self.backend.get(&id).await.map_err(unavailable)? else {
            let revoked = self.backend.is_revoked(&id).await.map_err(unavailable)?;
            return Err(if revoked {
                TokenRejection::Revoked
            } else {
                TokenRejection::Unknown
            });
        };
        if grant.expires_at <= now {
            self.backend.remove(&id).await.map_err(unavailable)?;
            return Err(TokenRejection::Expired);
        }
        Ok(grant)
    }

    /// Revoke a token by id: its grant (and so its credential) is dropped and
    /// the id remembered until its expiry, so the token then resolves to
    /// [`TokenRejection::Revoked`]. Returns whether a live grant existed.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the backend fails.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyRegistry`].
    pub async fn revoke(&self, id: &str) -> Result<bool, AuthProxyError> {
        self.backend.revoke(id).await
    }

    /// Revoke every token of a run, as [`AuthProxyRegistry::revoke`] does for
    /// one. Returns how many were revoked.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the backend fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::AuthProxyRegistry;
    ///
    /// # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// assert_eq!(AuthProxyRegistry::default().revoke_run("run-1").await?, 0);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn revoke_run(&self, run_id: &str) -> Result<usize, AuthProxyError> {
        self.backend.revoke_run(run_id).await
    }

    /// Drop every grant expired at `now`, and the revocations of tokens
    /// expired at `now`. Returns how many grants were dropped.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the backend fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::AuthProxyRegistry;
    ///
    /// # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// assert_eq!(AuthProxyRegistry::default().purge_expired(1_700_000_000).await?, 0);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn purge_expired(&self, now: u64) -> Result<usize, AuthProxyError> {
        self.backend.purge_expired(now).await
    }

    /// Number of live grants.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the backend fails.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyRegistry`].
    pub async fn len(&self) -> Result<usize, AuthProxyError> {
        self.backend.len().await
    }

    /// Whether no grant is held.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the backend fails.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyRegistry`].
    pub async fn is_empty(&self) -> Result<bool, AuthProxyError> {
        self.backend.is_empty().await
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
    use serde_json::{from_value, json};

    use super::super::secret::{HostPattern, SecretCredential, SecretInjection};
    use super::super::{CredentialKind, ProxyCredential};
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
            )
            .into(),
        }
    }

    fn invalid_message(result: Result<IssuedToken, AuthProxyError>) -> String {
        match result {
            Err(AuthProxyError::InvalidRequest(message)) => message,
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn issue_then_resolve_returns_grant() {
        let registry = AuthProxyRegistry::default();
        let issued = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
        let grant = registry.resolve(&issued.token, NOW + 1).await.unwrap();
        assert_eq!(grant.run_id, "run-1");
        assert_eq!(grant.step, "review");
        assert_eq!(grant.expires_at, NOW + 600);
        assert_eq!(grant.id, issued.id);
        assert_eq!(grant.credential.expose(), "sk-ant-oat01-test");
        match grant.credential {
            GrantCredential::Claude(claude) => {
                assert_eq!(claude.kind(), CredentialKind::OauthToken)
            }
            other => panic!("expected a Claude credential, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_unknown_token_is_rejected() {
        let registry = AuthProxyRegistry::default();
        registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
        assert_eq!(
            registry.resolve("invalide", NOW).await.unwrap_err(),
            TokenRejection::Unknown
        );
    }

    #[tokio::test]
    async fn resolve_expired_token_is_rejected_and_removed() {
        let registry = AuthProxyRegistry::default();
        let issued = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
        assert_eq!(
            registry
                .resolve(&issued.token, NOW + 600)
                .await
                .unwrap_err(),
            TokenRejection::Expired
        );
        assert!(registry.is_empty().await.unwrap());
        assert_eq!(
            registry
                .resolve(&issued.token, NOW + 600)
                .await
                .unwrap_err(),
            TokenRejection::Unknown
        );
    }

    #[tokio::test]
    async fn revoked_token_resolves_to_revoked() {
        let registry = AuthProxyRegistry::default();
        let issued = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
        assert!(registry.revoke(&issued.id).await.unwrap());
        assert!(!registry.revoke(&issued.id).await.unwrap());
        assert_eq!(
            registry.resolve(&issued.token, NOW).await.unwrap_err(),
            TokenRejection::Revoked
        );
    }

    #[tokio::test]
    async fn revoke_run_only_drops_that_run() {
        let registry = AuthProxyRegistry::default();
        let a1 = registry
            .issue(request("run-a", NOW + 600), NOW)
            .await
            .unwrap();
        let a2 = registry
            .issue(request("run-a", NOW + 600), NOW)
            .await
            .unwrap();
        let b = registry
            .issue(request("run-b", NOW + 600), NOW)
            .await
            .unwrap();
        assert_eq!(registry.revoke_run("run-a").await.unwrap(), 2);
        assert_eq!(registry.revoke_run("run-a").await.unwrap(), 0);
        assert_eq!(
            registry.resolve(&a1.token, NOW).await.unwrap_err(),
            TokenRejection::Revoked
        );
        assert_eq!(
            registry.resolve(&a2.token, NOW).await.unwrap_err(),
            TokenRejection::Revoked
        );
        assert_eq!(
            registry.resolve(&b.token, NOW).await.unwrap().run_id,
            "run-b"
        );
    }

    #[tokio::test]
    async fn purge_expired_counts() {
        let registry = AuthProxyRegistry::default();
        registry
            .issue(request("run-1", NOW + 10), NOW)
            .await
            .unwrap();
        registry
            .issue(request("run-1", NOW + 20), NOW)
            .await
            .unwrap();
        let live = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
        assert_eq!(registry.purge_expired(NOW + 20).await.unwrap(), 2);
        assert_eq!(registry.len().await.unwrap(), 1);
        assert!(registry.resolve(&live.token, NOW + 20).await.is_ok());
        assert_eq!(registry.purge_expired(NOW + 20).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn issue_rejects_past_expiry() {
        let registry = AuthProxyRegistry::default();
        let message = invalid_message(registry.issue(request("run-1", NOW), NOW).await);
        assert_eq!(message, "expires_at is in the past");
        let message = invalid_message(registry.issue(request("run-1", NOW - 1), NOW).await);
        assert_eq!(message, "expires_at is in the past");
        assert!(registry.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn issue_rejects_lifetime_over_24h() {
        let registry = AuthProxyRegistry::default();
        let max = MAX_TOKEN_LIFETIME.as_secs();
        let message = invalid_message(registry.issue(request("run-1", NOW + max + 1), NOW).await);
        assert!(message.contains("24 hours"), "{message}");
        assert!(
            registry
                .issue(request("run-1", NOW + max), NOW)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn issue_rejects_empty_credential() {
        let registry = AuthProxyRegistry::default();
        let mut req = request("run-1", NOW + 600);
        req.credential = ProxyCredential::new(CredentialKind::ApiKey, String::new()).into();
        assert_eq!(
            invalid_message(registry.issue(req, NOW).await),
            "credential is empty"
        );

        let message = invalid_message(registry.issue(request("", NOW + 600), NOW).await);
        assert_eq!(message, "run_id is empty");

        let mut req = request("run-1", NOW + 600);
        req.step = String::new();
        assert_eq!(
            invalid_message(registry.issue(req, NOW).await),
            "step is empty"
        );

        let mut req = request("run-1", NOW + 600);
        req.credential =
            ProxyCredential::new(CredentialKind::OauthToken, "tok\nen".to_string()).into();
        let message = invalid_message(registry.issue(req, NOW).await);
        assert!(message.contains("header cannot carry"), "{message}");
        assert!(registry.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn token_has_prefix_and_never_starts_with_sk_ant() {
        let registry = AuthProxyRegistry::default();
        let issued = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
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

    #[tokio::test]
    async fn two_tokens_differ() {
        let registry = AuthProxyRegistry::default();
        let a = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
        let b = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
        assert_ne!(a.token, b.token);
        assert_ne!(a.id, b.id);
        assert_eq!(registry.len().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn registry_with_backend_shares_grants_between_clones() {
        let backend = Arc::new(MemoryGrantBackend::default());
        let a = AuthProxyRegistry::with_backend(backend.clone());
        let b = AuthProxyRegistry::with_backend(backend.clone());
        let issued = a.issue(request("run-1", NOW + 600), NOW).await.unwrap();

        let grant = b.resolve(&issued.token, NOW + 1).await.unwrap();
        assert_eq!(grant.id, issued.id);
        assert_eq!(grant.run_id, "run-1");
        assert_eq!(grant.credential.expose(), "sk-ant-oat01-test");
        assert_eq!(b.len().await.unwrap(), 1);

        assert!(b.revoke(&issued.id).await.unwrap());
        assert_eq!(
            a.resolve(&issued.token, NOW + 1).await.unwrap_err(),
            TokenRejection::Revoked
        );
        assert!(a.is_empty().await.unwrap());
    }

    #[test]
    fn registry_debug_shows_no_grant() {
        let debug = format!("{:?}", AuthProxyRegistry::default());
        assert_eq!(debug, "AuthProxyRegistry { .. }");
    }

    #[tokio::test]
    async fn issued_token_debug_redacts_token() {
        let registry = AuthProxyRegistry::default();
        let issued = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
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

    fn secret_request(run_id: &str, injection: SecretInjection, hosts: &[&str]) -> TokenRequest {
        TokenRequest {
            run_id: run_id.to_string(),
            step: "review".to_string(),
            expires_at: NOW + 600,
            credential: SecretCredential::new(
                "GITHUB_TOKEN".to_string(),
                "ghp_test".to_string(),
                injection,
                hosts
                    .iter()
                    .map(|h| HostPattern::parse(h).unwrap())
                    .collect(),
            )
            .into(),
        }
    }

    #[tokio::test]
    async fn revoke_run_tombstones_every_grant() {
        let registry = AuthProxyRegistry::default();
        let claude = registry
            .issue(request("run-a", NOW + 600), NOW)
            .await
            .unwrap();
        let secret = registry
            .issue(
                secret_request("run-a", SecretInjection::Bearer, &["api.github.com"]),
                NOW,
            )
            .await
            .unwrap();
        assert_eq!(registry.revoke_run("run-a").await.unwrap(), 2);
        for token in [&claude.token, &secret.token] {
            assert_eq!(
                registry.resolve(token, NOW).await.unwrap_err(),
                TokenRejection::Revoked
            );
        }
        assert!(registry.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn purge_drops_expired_tombstones() {
        let registry = AuthProxyRegistry::default();
        let issued = registry
            .issue(request("run-1", NOW + 600), NOW)
            .await
            .unwrap();
        assert!(registry.revoke(&issued.id).await.unwrap());
        assert_eq!(registry.purge_expired(NOW + 599).await.unwrap(), 0);
        assert_eq!(
            registry.resolve(&issued.token, NOW).await.unwrap_err(),
            TokenRejection::Revoked
        );
        assert_eq!(registry.purge_expired(NOW + 600).await.unwrap(), 0);
        assert_eq!(
            registry.resolve(&issued.token, NOW).await.unwrap_err(),
            TokenRejection::Unknown
        );
    }

    #[tokio::test]
    async fn issue_secret_with_empty_hosts_is_invalid() {
        let registry = AuthProxyRegistry::default();
        let req = secret_request("run-1", SecretInjection::Bearer, &[]);
        let message = invalid_message(registry.issue(req, NOW).await);
        assert!(message.contains("empty host allowlist"), "{message}");
        assert!(registry.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn issue_secret_with_bad_host_is_invalid() {
        let body = json!({
            "run_id": "run-1",
            "step": "review",
            "expires_at": NOW + 600,
            "credential": {
                "name": "GITHUB_TOKEN",
                "value": "ghp_test",
                "injection": "bearer",
                "hosts": [".*github.com"]
            }
        });
        assert!(from_value::<TokenRequest>(body).is_err());

        let registry = AuthProxyRegistry::default();
        let req = secret_request(
            "run-1",
            SecretInjection::Header("Host".to_string()),
            &["api.github.com"],
        );
        let message = invalid_message(registry.issue(req, NOW).await);
        assert!(message.contains("managed by the proxy"), "{message}");

        let mut req = secret_request("run-1", SecretInjection::Bearer, &["api.github.com"]);
        req.credential = SecretCredential::new(
            "GITHUB_TOKEN".to_string(),
            "ghp test".to_string(),
            SecretInjection::Bearer,
            vec![HostPattern::parse("api.github.com").unwrap()],
        )
        .into();
        let message = invalid_message(registry.issue(req, NOW).await);
        assert!(!message.contains("ghp test"), "{message}");
        assert!(registry.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn issue_then_resolve_secret_grant() {
        let registry = AuthProxyRegistry::default();
        let injection = SecretInjection::Basic {
            username: "oauth2".to_string(),
        };
        let issued = registry
            .issue(
                secret_request("run-1", injection.clone(), &["gitlab.com"]),
                NOW,
            )
            .await
            .unwrap();
        let grant = registry.resolve(&issued.token, NOW + 1).await.unwrap();
        assert_eq!(grant.run_id, "run-1");
        match grant.credential {
            GrantCredential::Secret(secret) => {
                assert_eq!(secret.name(), "GITHUB_TOKEN");
                assert_eq!(secret.expose(), "ghp_test");
                assert_eq!(secret.injection(), &injection);
                assert!(secret.allows_host("gitlab.com"));
            }
            other => panic!("expected a secret, got {other:?}"),
        }
    }
}
