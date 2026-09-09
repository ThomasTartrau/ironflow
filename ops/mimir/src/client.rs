//! [`MimirClient`] built from an [`OperationContext`]'s secret store.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use ironflow_ops_common::HttpApiClient;
use reqwest::Client;
use reqwest::RequestBuilder;

/// A Grafana Mimir client that resolves credentials from the workflow's secret store.
///
/// Wraps a base URL and optional authentication, providing convenience methods
/// to build authenticated HTTP requests against the Mimir API. Mimir exposes a
/// Prometheus-compatible query API, so this client works with any
/// Prometheus-compatible backend.
///
/// # Construction
///
/// Use [`from_context`](MimirClient::from_context) to read the connection details
/// from the workflow's secret store:
///
/// - `mimir_url` (required) -- the base URL of the Mimir instance (e.g. `http://mimir:8080`).
/// - `mimir_token` (optional) -- a Bearer token for authentication.
/// - `mimir_basic_auth` (optional) -- basic auth credentials as `user:password`.
///
/// If neither `mimir_token` nor `mimir_basic_auth` is set, requests are sent
/// without authentication.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::MimirClient;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct MimirClient {
    inner: HttpApiClient,
}

impl fmt::Debug for MimirClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MimirClient")
            .field("base_url", &self.inner.base_url())
            .field("auth", self.inner.auth())
            .field("http", &"[reqwest::Client]")
            .finish()
    }
}

impl MimirClient {
    /// Build a client from an [`OperationContext`].
    ///
    /// Reads `mimir_url` (required), `mimir_token` and `mimir_basic_auth`
    /// (both optional) from the secret store.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store itself fails
    /// or if `mimir_url` is missing or empty.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::MimirClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let mimir = MimirClient::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let inner =
            HttpApiClient::from_context(ctx, "mimir_url", "mimir_token", "mimir_basic_auth")
                .await?;
        Ok(Self { inner })
    }

    /// Build a client from explicit parameters, without a secret store.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::MimirClient;
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
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
    /// use ironflow_ops_mimir::MimirClient;
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new())
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
    /// use ironflow_ops_mimir::MimirClient;
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new())
    ///     .with_basic_auth("user", "password");
    /// ```
    #[must_use]
    pub fn with_basic_auth(mut self, user: &str, password: &str) -> Self {
        self.inner = self.inner.with_basic_auth(user, password);
        self
    }

    /// The base URL of the Mimir instance.
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
    async fn from_context_fails_without_mimir_url() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = MimirClient::from_context(&ctx).await.unwrap_err();
        match err {
            OperationError::Secret { message } => {
                assert!(
                    message.contains("mimir_url"),
                    "expected mimir_url error, got: {message}"
                );
            }
            other => panic!("expected Secret error, got: {other}"),
        }
    }

    #[test]
    fn new_trims_trailing_slash() {
        let mimir = MimirClient::new("http://mimir:8080/", Client::new());
        assert_eq!(mimir.base_url(), "http://mimir:8080");
    }

    #[test]
    fn with_bearer_token_sets_auth() {
        let mimir = MimirClient::new("http://mimir:8080", Client::new()).with_bearer_token("tok");
        let debug = format!("{mimir:?}");
        assert!(debug.contains("Bearer(<redacted>)"));
    }

    #[test]
    fn with_basic_auth_sets_auth() {
        let mimir =
            MimirClient::new("http://mimir:8080", Client::new()).with_basic_auth("user", "pass");
        let debug = format!("{mimir:?}");
        assert!(debug.contains("Basic"));
        assert!(debug.contains("user"));
    }

    #[test]
    fn debug_does_not_leak_token() {
        let mimir = MimirClient::new("http://mimir:8080", Client::new())
            .with_bearer_token("super-secret-token");
        let debug = format!("{mimir:?}");
        assert!(
            !debug.contains("super-secret-token"),
            "Debug output must not contain the bearer token: {debug}"
        );
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn debug_does_not_leak_password() {
        let mimir = MimirClient::new("http://mimir:8080", Client::new())
            .with_basic_auth("admin", "super-secret-password");
        let debug = format!("{mimir:?}");
        assert!(
            !debug.contains("super-secret-password"),
            "Debug output must not contain the password: {debug}"
        );
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("admin"));
    }
}
