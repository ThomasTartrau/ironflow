//! The real credential the proxy injects, and how the worker picks it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::account::{AccountCredential, AccountSession, ClaudeSubscriptionKind};

use super::AuthProxyError;

/// Environment variable of an Anthropic API key.
const API_KEY_ENV: &str = "ANTHROPIC_API_KEY";

/// Environment variable of a Claude subscription OAuth token.
const OAUTH_TOKEN_ENV: &str = ClaudeSubscriptionKind::TOKEN_ENV;

/// How the proxy presents a credential to the Anthropic API.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::CredentialKind;
///
/// # fn example() -> Result<(), serde_json::Error> {
/// assert_eq!(serde_json::to_string(&CredentialKind::OauthToken)?, "\"oauth_token\"");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// A Claude subscription OAuth token, sent as `Authorization: Bearer`.
    OauthToken,
    /// An Anthropic API key, sent as `x-api-key`.
    ApiKey,
}

/// A real Claude credential. Its [`Debug`] output never shows the value.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{CredentialKind, ProxyCredential};
///
/// let credential = ProxyCredential::new(CredentialKind::ApiKey, "sk-ant-api03-x".to_string());
/// assert_eq!(credential.kind(), CredentialKind::ApiKey);
/// assert!(!format!("{credential:?}").contains("sk-ant"));
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct ProxyCredential {
    kind: CredentialKind,
    value: String,
}

impl ProxyCredential {
    /// Wrap a credential value.
    ///
    /// # Examples
    ///
    /// See [`ProxyCredential`].
    pub fn new(kind: CredentialKind, value: String) -> Self {
        Self { kind, value }
    }

    /// How the credential is presented upstream.
    ///
    /// # Examples
    ///
    /// See [`ProxyCredential`].
    pub fn kind(&self) -> CredentialKind {
        self.kind
    }

    /// The raw credential value. Never log it.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::{CredentialKind, ProxyCredential};
    ///
    /// let credential = ProxyCredential::new(CredentialKind::OauthToken, "v".to_string());
    /// assert_eq!(credential.expose(), "v");
    /// ```
    pub fn expose(&self) -> &str {
        &self.value
    }

    /// Convert a Provider Account credential: `ANTHROPIC_API_KEY` becomes an
    /// [`CredentialKind::ApiKey`], anything else an [`CredentialKind::OauthToken`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::account::AccountCredential;
    /// use ironflow_core::auth_proxy::{CredentialKind, ProxyCredential};
    ///
    /// let account = AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", "t".to_string());
    /// assert_eq!(ProxyCredential::from_account(&account).kind(), CredentialKind::OauthToken);
    /// ```
    pub fn from_account(credential: &AccountCredential) -> Self {
        let kind = if credential.env_var() == API_KEY_ENV {
            CredentialKind::ApiKey
        } else {
            CredentialKind::OauthToken
        };
        Self::new(kind, credential.expose().to_string())
    }
}

impl fmt::Debug for ProxyCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyCredential")
            .field("kind", &self.kind)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Pick the credential the proxy injects for a step: the Provider Account
/// session first, else a non-empty `CLAUDE_CODE_OAUTH_TOKEN`, else a non-empty
/// `ANTHROPIC_API_KEY`, both read through `env`.
///
/// # Errors
///
/// Returns [`AuthProxyError::NoCredential`] when none is available.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{CredentialKind, resolve_credential};
///
/// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let credential = resolve_credential(None, |name| {
///     (name == "ANTHROPIC_API_KEY").then(|| "sk-ant-api03-x".to_string())
/// })?;
/// assert_eq!(credential.kind(), CredentialKind::ApiKey);
/// # Ok(())
/// # }
/// ```
pub fn resolve_credential(
    account: Option<&AccountSession>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<ProxyCredential, AuthProxyError> {
    if let Some(session) = account {
        return Ok(ProxyCredential::from_account(session.credential()));
    }
    let from_env = |name: &str, kind: CredentialKind| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(|value| ProxyCredential::new(kind, value))
    };
    from_env(OAUTH_TOKEN_ENV, CredentialKind::OauthToken)
        .or_else(|| from_env(API_KEY_ENV, CredentialKind::ApiKey))
        .ok_or(AuthProxyError::NoCredential)
}

#[cfg(test)]
mod tests {
    use crate::account::RateLimitRecorder;

    use super::*;

    fn session(env_var: &'static str, value: &str) -> AccountSession {
        AccountSession::new(
            AccountCredential::new(env_var, value.to_string()),
            RateLimitRecorder::default(),
        )
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn resolve_prefers_account() {
        let account = session("CLAUDE_CODE_OAUTH_TOKEN", "account-token");
        let env = env_of(&[
            ("CLAUDE_CODE_OAUTH_TOKEN", "env-token"),
            ("ANTHROPIC_API_KEY", "env-key"),
        ]);
        let credential = resolve_credential(Some(&account), env).unwrap();
        assert_eq!(credential.kind(), CredentialKind::OauthToken);
        assert_eq!(credential.expose(), "account-token");
    }

    #[test]
    fn resolve_falls_back_to_oauth_env_then_api_key() {
        let both = env_of(&[
            ("CLAUDE_CODE_OAUTH_TOKEN", "env-token"),
            ("ANTHROPIC_API_KEY", "env-key"),
        ]);
        let credential = resolve_credential(None, both).unwrap();
        assert_eq!(credential.kind(), CredentialKind::OauthToken);
        assert_eq!(credential.expose(), "env-token");

        let key_only = env_of(&[("ANTHROPIC_API_KEY", "env-key")]);
        let credential = resolve_credential(None, key_only).unwrap();
        assert_eq!(credential.kind(), CredentialKind::ApiKey);
        assert_eq!(credential.expose(), "env-key");
    }

    #[test]
    fn resolve_errors_without_credential() {
        let err = resolve_credential(None, env_of(&[])).unwrap_err();
        assert!(matches!(err, AuthProxyError::NoCredential), "{err}");
    }

    #[test]
    fn resolve_ignores_empty_env() {
        let env = env_of(&[
            ("CLAUDE_CODE_OAUTH_TOKEN", ""),
            ("ANTHROPIC_API_KEY", "env-key"),
        ]);
        let credential = resolve_credential(None, env).unwrap();
        assert_eq!(credential.kind(), CredentialKind::ApiKey);

        let both_empty = [("CLAUDE_CODE_OAUTH_TOKEN", ""), ("ANTHROPIC_API_KEY", "")];
        let result = resolve_credential(None, env_of(&both_empty));
        assert!(matches!(result, Err(AuthProxyError::NoCredential)));
    }

    #[test]
    fn debug_redacts_value() {
        let value = "sk-ant-oat01-secret";
        let credential = ProxyCredential::new(CredentialKind::OauthToken, value.to_string());
        let debug = format!("{credential:?}");
        assert!(!debug.contains("sk-ant-oat01-secret"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(debug.contains("OauthToken"), "{debug}");
    }

    #[test]
    fn from_account_maps_kinds() {
        let oauth = AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", "t".to_string());
        assert_eq!(
            ProxyCredential::from_account(&oauth).kind(),
            CredentialKind::OauthToken
        );
        let key = AccountCredential::new("ANTHROPIC_API_KEY", "k".to_string());
        let converted = ProxyCredential::from_account(&key);
        assert_eq!(converted.kind(), CredentialKind::ApiKey);
        assert_eq!(converted.expose(), "k");
    }
}
