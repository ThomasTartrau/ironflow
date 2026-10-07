//! Auth proxy: the agent pod never holds the Claude credential.
//!
//! With `K8sEphemeralProvider::auth_proxy` (feature `transport-k8s`), the worker asks the
//! `ironflow-auth-proxy` service for an opaque token bound to one run and one
//! step, and the agent pod receives only that token and the proxy URL
//! ([`POD_BASE_URL_ENV`], [`POD_TOKEN_ENV`]). Claude Code sends the opaque
//! token to the proxy, which swaps it for the real credential and relays the
//! request to `api.anthropic.com`. At the end of the step the worker revokes
//! the token; its expiry is the backstop.
//!
//! Proxied secrets ([`ProxiedSecret`]) extend this to other credentials: the
//! pod receives an opaque token per secret and sends its requests to
//! `<proxy>/r/<host>/<path>`. The proxy checks `<host>` against the secret's
//! allowlist ([`HostPattern`]), injects the real secret as configured by
//! [`SecretInjection`] and relays to `https://<host>/<path>`.
//!
//! This module holds what both sides share:
//!
//! * [`AuthProxyRegistry`] - the opaque token registry (proxy side);
//! * [`GrantBackend`] - where the registry keeps its grants, and
//!   [`MemoryGrantBackend`], the in-process default;
//! * [`extract_opaque_token`], [`is_allowed_path`], [`upstream_headers`],
//!   [`downstream_headers`] - the relay policy (proxy side);
//! * [`resolve_credential`] / [`ProxyCredential`] - which credential the
//!   worker hands to the proxy;
//! * [`GrantCredential`], [`SecretCredential`], [`is_relay_path`],
//!   [`secret_upstream_headers`] - proxied secrets and their relay under
//!   [`RELAY_PREFIX`];
//! * [`AuthProxyClient`] - the admin client the worker uses to issue and
//!   revoke tokens.
//!
//! # Examples
//!
//! ```
//! use ironflow_core::auth_proxy::{
//!     AuthProxyRegistry, CredentialKind, ProxyCredential, TokenRequest,
//! };
//!
//! # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
//! let registry = AuthProxyRegistry::default();
//! let issued = registry
//!     .issue(
//!         TokenRequest {
//!             run_id: "run-1".to_string(),
//!             step: "review".to_string(),
//!             expires_at: 1_000 + 600,
//!             credential: ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-x".to_string()).into(),
//!         },
//!         1_000,
//!     )
//!     .await?;
//! assert!(issued.token.starts_with("ifap_"));
//! let step = registry.resolve(&issued.token, 1_001).await.map(|grant| grant.step);
//! assert_eq!(step, Ok("review".to_string()));
//! # Ok(())
//! # }
//! ```

mod backend;
mod client;
mod credential;
mod policy;
mod registry;
mod secret;

use std::time::Duration;

use thiserror::Error;

pub use backend::{GrantBackend, GrantFuture, MemoryGrantBackend};
pub use client::AuthProxyClient;
pub use credential::{CredentialKind, ProxyCredential, resolve_credential};
pub use policy::{
    downstream_headers, error_body, extract_opaque_token, is_allowed_method, is_allowed_path,
    is_relay_method, is_relay_path, secret_upstream_headers, upstream_headers,
};
pub use registry::{
    AuthProxyRegistry, Grant, IssuedToken, TokenRejection, TokenRequest, admin_key_matches,
    token_id,
};
pub use secret::{
    GrantCredential, HostPattern, ProxiedSecret, SecretCredential, SecretInjection,
    is_valid_request_host,
};

/// The upstream of the Claude relay (`/v1/`). Proxied secrets (under
/// [`RELAY_PREFIX`]) reach their own allowlisted https hosts.
pub const DEFAULT_UPSTREAM: &str = "https://api.anthropic.com";

/// Environment variable holding the admin key shared by the worker and the proxy.
pub const ADMIN_KEY_ENV: &str = "IRONFLOW_AUTH_PROXY_ADMIN_KEY";

/// Prefix of every opaque token. It never looks like an Anthropic credential.
pub const TOKEN_PREFIX: &str = "ifap_";

/// Longest lifetime a token may be issued for.
pub const MAX_TOKEN_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// `anthropic-beta` flag required by the API when authenticating with an OAuth token.
pub const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Environment variable carrying the opaque token in the agent pod.
pub const POD_TOKEN_ENV: &str = "ANTHROPIC_AUTH_TOKEN";

/// Environment variable carrying the proxy URL in the agent pod.
pub const POD_BASE_URL_ENV: &str = "ANTHROPIC_BASE_URL";

/// Path prefix of the proxied secret relay: `<proxy>/r/<host>/<path>`.
pub const RELAY_PREFIX: &str = "/r";

/// Suffix of the pod environment variable carrying the relay base URL of a
/// proxied secret: `<env>_URL`.
pub const SECRET_URL_SUFFIX: &str = "_URL";

/// Errors of the auth proxy. No variant ever carries a credential or token value.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::AuthProxyError;
///
/// let err = AuthProxyError::Admin { status: 401, message: "denied".to_string() };
/// assert!(err.to_string().contains("401"));
/// ```
#[derive(Debug, Error)]
pub enum AuthProxyError {
    /// Neither a Provider Account nor the worker environment carries a credential.
    #[error(
        "no credential: attach a Provider Account or set CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY"
    )]
    NoCredential,
    /// The token request is invalid.
    #[error("invalid token request: {0}")]
    InvalidRequest(String),
    /// The system random source failed.
    #[error("random source failed: {0}")]
    Random(String),
    /// The admin API answered with an unexpected status.
    #[error("auth proxy admin API returned HTTP {status}: {message}")]
    Admin {
        /// HTTP status returned.
        status: u16,
        /// Response body, truncated.
        message: String,
    },
    /// The admin API could not be reached.
    #[error("auth proxy unreachable: {0}")]
    Transport(String),
    /// The [`GrantBackend`] holding the grants failed. The message never
    /// carries a token or credential value.
    #[error("auth proxy registry backend failed: {0}")]
    Backend(String),
}
