//! [`LokiClient`] built from an [`OperationContext`]'s secret store.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use ironflow_ops_common::HttpApiClient;
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
    inner: HttpApiClient,
}

impl fmt::Debug for LokiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LokiClient")
            .field("base_url", &self.inner.base_url())
            .field("auth", self.inner.auth())
            .field("http", &"[reqwest::Client]")
            .finish()
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
        let inner =
            HttpApiClient::from_context(ctx, "loki_url", "loki_token", "loki_basic_auth").await?;
        Ok(Self { inner })
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
            inner: HttpApiClient::new(base_url, http),
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
        self.inner = self.inner.with_bearer_token(token);
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
        self.inner = self.inner.with_basic_auth(user, password);
        self
    }

    /// The base URL of the Loki instance.
    pub fn base_url(&self) -> &str {
        self.inner.base_url()
    }

    /// The underlying HTTP client.
    pub fn http_client(&self) -> &Client {
        self.inner.http_client()
    }

    /// Build a GET request to the given path, with authentication applied.
    pub(crate) fn get(&self, path: &str) -> RequestBuilder {
        self.inner.get(path)
    }

    /// Build a POST request to the given path, with authentication applied.
    pub(crate) fn post(&self, path: &str) -> RequestBuilder {
        self.inner.post(path)
    }

    /// Build a DELETE request to the given path, with authentication applied.
    pub(crate) fn delete(&self, path: &str) -> RequestBuilder {
        self.inner.delete(path)
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
        let debug = format!("{loki:?}");
        assert!(debug.contains("Bearer(<redacted>)"));
    }

    #[test]
    fn with_basic_auth_sets_auth() {
        let loki =
            LokiClient::new("http://loki:3100", Client::new()).with_basic_auth("user", "pass");
        let debug = format!("{loki:?}");
        assert!(debug.contains("Basic"));
        assert!(debug.contains("user"));
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
