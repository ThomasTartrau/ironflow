//! [`LokiClient`] built from an [`OperationContext`]'s secret store.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use reqwest::Client;
use reqwest::RequestBuilder;

/// A Grafana Loki client that resolves credentials from the workflow's secret store.
///
/// Wraps a base URL and optional authentication, providing convenience methods
/// to build authenticated HTTP requests against the Loki API.
///
/// # Construction
///
/// Use [`from_context`](LokiClient::from_context) to read the connection details
/// from the workflow's secret store:
///
/// - `loki_url` (required) -- the base URL of the Loki instance (e.g. `http://loki:3100`).
/// - `loki_token` (optional) -- a Bearer token for authentication.
/// - `loki_basic_auth` (optional) -- basic auth credentials as `user:password`.
///
/// If neither `loki_token` nor `loki_basic_auth` is set, requests are sent
/// without authentication (Loki can run without auth).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::LokiClient;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct LokiClient {
    base_url: String,
    auth: Auth,
    http: Client,
}

impl fmt::Debug for LokiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LokiClient")
            .field("base_url", &self.base_url)
            .field("auth", &self.auth)
            .field("http", &"[reqwest::Client]")
            .finish()
    }
}

#[derive(Clone)]
enum Auth {
    None,
    Bearer(String),
    Basic { user: String, password: String },
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::None => write!(f, "None"),
            Auth::Bearer(_) => write!(f, "Bearer(<redacted>)"),
            Auth::Basic { user, .. } => {
                write!(f, "Basic {{ user: {user:?}, password: <redacted> }}")
            }
        }
    }
}

impl LokiClient {
    /// Build a client from an [`OperationContext`].
    ///
    /// Reads `loki_url` (required), `loki_token` and `loki_basic_auth`
    /// (both optional) from the secret store.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store itself fails
    /// or if `loki_url` is missing or empty.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::LokiClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let loki = LokiClient::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let url_secret = ctx.secrets().get("loki_url").await?;
        let base_url = url_secret
            .filter(|s| !s.value.is_empty())
            .map(|s| s.value)
            .ok_or_else(|| OperationError::Secret {
                message: "missing or empty secret: loki_url".into(),
            })?;

        let base_url = base_url.trim_end_matches('/').to_owned();

        let token = ctx.secrets().get("loki_token").await?;
        let basic = ctx.secrets().get("loki_basic_auth").await?;

        let auth = if let Some(t) = token.filter(|s| !s.value.is_empty()) {
            Auth::Bearer(t.value)
        } else if let Some(b) = basic.filter(|s| !s.value.is_empty()) {
            let (user, password) =
                b.value
                    .split_once(':')
                    .ok_or_else(|| OperationError::Secret {
                        message: "loki_basic_auth must be in 'user:password' format".into(),
                    })?;
            Auth::Basic {
                user: user.to_owned(),
                password: password.to_owned(),
            }
        } else {
            Auth::None
        };

        Ok(Self {
            base_url,
            auth,
            http: ctx.http_client().clone(),
        })
    }

    /// Build a client from explicit parameters, without a secret store.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::LokiClient;
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// ```
    pub fn new(base_url: &str, http: Client) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            auth: Auth::None,
            http,
        }
    }

    /// Set Bearer token authentication.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::LokiClient;
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new())
    ///     .with_bearer_token("my-token");
    /// ```
    #[must_use]
    pub fn with_bearer_token(mut self, token: &str) -> Self {
        self.auth = Auth::Bearer(token.to_owned());
        self
    }

    /// Set basic authentication.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::LokiClient;
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new())
    ///     .with_basic_auth("user", "password");
    /// ```
    #[must_use]
    pub fn with_basic_auth(mut self, user: &str, password: &str) -> Self {
        self.auth = Auth::Basic {
            user: user.to_owned(),
            password: password.to_owned(),
        };
        self
    }

    /// The base URL of the Loki instance.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The underlying HTTP client.
    pub fn http_client(&self) -> &Client {
        &self.http
    }

    /// Build a GET request to the given path, with authentication applied.
    pub(crate) fn get(&self, path: &str) -> RequestBuilder {
        self.authenticate(self.http.get(format!("{}{path}", self.base_url)))
    }

    /// Build a POST request to the given path, with authentication applied.
    pub(crate) fn post(&self, path: &str) -> RequestBuilder {
        self.authenticate(self.http.post(format!("{}{path}", self.base_url)))
    }

    /// Build a DELETE request to the given path, with authentication applied.
    pub(crate) fn delete(&self, path: &str) -> RequestBuilder {
        self.authenticate(self.http.delete(format!("{}{path}", self.base_url)))
    }

    fn authenticate(&self, builder: RequestBuilder) -> RequestBuilder {
        match &self.auth {
            Auth::None => builder,
            Auth::Bearer(token) => builder.bearer_auth(token),
            Auth::Basic { user, password } => builder.basic_auth(user, Some(password)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    use super::*;

    #[tokio::test]
    async fn from_context_fails_without_loki_url() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = LokiClient::from_context(&ctx).await.unwrap_err();
        match err {
            OperationError::Secret { message } => {
                assert!(
                    message.contains("loki_url"),
                    "expected loki_url error, got: {message}"
                );
            }
            other => panic!("expected Secret error, got: {other}"),
        }
    }

    #[test]
    fn new_trims_trailing_slash() {
        let loki = LokiClient::new("http://loki:3100/", Client::new());
        assert_eq!(loki.base_url(), "http://loki:3100");
    }

    #[test]
    fn with_bearer_token_sets_auth() {
        let loki = LokiClient::new("http://loki:3100", Client::new()).with_bearer_token("tok");
        assert!(matches!(loki.auth, Auth::Bearer(ref t) if t == "tok"));
    }

    #[test]
    fn with_basic_auth_sets_auth() {
        let loki =
            LokiClient::new("http://loki:3100", Client::new()).with_basic_auth("user", "pass");
        assert!(
            matches!(loki.auth, Auth::Basic { ref user, ref password } if user == "user" && password == "pass")
        );
    }

    #[test]
    fn debug_does_not_leak_token() {
        let loki = LokiClient::new("http://loki:3100", Client::new())
            .with_bearer_token("super-secret-token");
        let debug = format!("{loki:?}");
        assert!(
            !debug.contains("super-secret-token"),
            "Debug output must not contain the bearer token: {debug}"
        );
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn debug_does_not_leak_password() {
        let loki = LokiClient::new("http://loki:3100", Client::new())
            .with_basic_auth("admin", "super-secret-password");
        let debug = format!("{loki:?}");
        assert!(
            !debug.contains("super-secret-password"),
            "Debug output must not contain the password: {debug}"
        );
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("admin"));
    }
}
