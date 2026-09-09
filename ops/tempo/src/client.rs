//! [`TempoClient`] built from an [`OperationContext`]'s secret store.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use ironflow_ops_common::HttpApiClient;
use reqwest::Client;
use reqwest::RequestBuilder;

/// A Grafana Tempo client that resolves credentials from the workflow's secret store.
///
/// Wraps a base URL and optional authentication, providing convenience methods
/// to build authenticated HTTP requests against the Tempo API.
///
/// # Construction
///
/// Use [`from_context`](TempoClient::from_context) to read the connection details
/// from the workflow's secret store:
///
/// - `tempo_url` (required) -- the base URL of the Tempo instance (e.g. `http://tempo:3200`).
/// - `tempo_token` (optional) -- a Bearer token for authentication.
/// - `tempo_basic_auth` (optional) -- basic auth credentials as `user:password`.
///
/// If neither `tempo_token` nor `tempo_basic_auth` is set, requests are sent
/// without authentication (Tempo can run without auth).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::TempoClient;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct TempoClient {
    inner: HttpApiClient,
}

impl fmt::Debug for TempoClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TempoClient")
            .field("base_url", &self.inner.base_url())
            .field("auth", self.inner.auth())
            .field("http", &"[reqwest::Client]")
            .finish()
    }
}

impl TempoClient {
    /// Build a client from an [`OperationContext`].
    ///
    /// Reads `tempo_url` (required), `tempo_token` and `tempo_basic_auth`
    /// (both optional) from the secret store.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store itself fails
    /// or if `tempo_url` is missing or empty.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::TempoClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let tempo = TempoClient::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let inner =
            HttpApiClient::from_context(ctx, "tempo_url", "tempo_token", "tempo_basic_auth")
                .await?;
        Ok(Self { inner })
    }

    /// Build a client from explicit parameters, without a secret store.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::TempoClient;
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
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
    /// use ironflow_ops_tempo::TempoClient;
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new())
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
    /// use ironflow_ops_tempo::TempoClient;
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new())
    ///     .with_basic_auth("user", "password");
    /// ```
    #[must_use]
    pub fn with_basic_auth(mut self, user: &str, password: &str) -> Self {
        self.inner = self.inner.with_basic_auth(user, password);
        self
    }

    /// The base URL of the Tempo instance.
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

    /// Build a PATCH request to the given path, with authentication applied.
    pub(crate) fn patch(&self, path: &str) -> RequestBuilder {
        self.inner.patch(path)
    }

    /// Build a DELETE request to the given path, with authentication applied.
    pub(crate) fn delete(&self, path: &str) -> RequestBuilder {
        self.inner.delete(path)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::error::OperationError;
    use ironflow_core::operation::{NoopSecretResolver, OperationContext};
    use wiremock::matchers::{header, header_exists, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn from_context_fails_without_tempo_url() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = TempoClient::from_context(&ctx).await.unwrap_err();
        match err {
            OperationError::Secret { message } => {
                assert!(
                    message.contains("tempo_url"),
                    "expected tempo_url error, got: {message}"
                );
            }
            other => panic!("expected Secret error, got: {other}"),
        }
    }

    #[test]
    fn new_trims_trailing_slash() {
        let tempo = TempoClient::new("http://tempo:3200/", Client::new());
        assert_eq!(tempo.base_url(), "http://tempo:3200");
    }

    #[test]
    fn with_bearer_token_sets_auth() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new()).with_bearer_token("tok");
        let debug = format!("{tempo:?}");
        assert!(debug.contains("Bearer(<redacted>)"));
    }

    #[test]
    fn with_basic_auth_sets_auth() {
        let tempo =
            TempoClient::new("http://tempo:3200", Client::new()).with_basic_auth("user", "pass");
        let debug = format!("{tempo:?}");
        assert!(debug.contains("Basic"));
        assert!(debug.contains("user"));
    }

    #[test]
    fn debug_does_not_leak_token() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new())
            .with_bearer_token("super-secret-token");
        let debug = format!("{tempo:?}");
        assert!(
            !debug.contains("super-secret-token"),
            "Debug output must not contain the bearer token: {debug}"
        );
        assert!(debug.contains("<redacted>"));
    }

    #[tokio::test]
    async fn bearer_auth_sends_authorization_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("authorization", "Bearer test-token-123"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let tempo =
            TempoClient::new(&server.uri(), Client::new()).with_bearer_token("test-token-123");
        let resp = tempo.get("/check").send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn basic_auth_sends_authorization_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header_exists("authorization"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let tempo =
            TempoClient::new(&server.uri(), Client::new()).with_basic_auth("admin", "secret");
        let resp = tempo.get("/check").send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn no_auth_sends_no_authorization_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let resp = tempo.get("/check").send().await.unwrap();
        assert_eq!(resp.status(), 200);

        let requests = server.received_requests().await.unwrap();
        assert!(
            !requests[0].headers.contains_key("authorization"),
            "no-auth client must not send authorization header"
        );
    }

    #[test]
    fn debug_does_not_leak_password() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new())
            .with_basic_auth("admin", "super-secret-password");
        let debug = format!("{tempo:?}");
        assert!(
            !debug.contains("super-secret-password"),
            "Debug output must not contain the password: {debug}"
        );
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("admin"));
    }
}
