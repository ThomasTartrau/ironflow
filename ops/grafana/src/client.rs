//! [`GrafanaClient`] built from an [`OperationContext`]'s secret store.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use ironflow_ops_common::HttpApiClient;
use ironflow_ops_common::helpers::{check_response_json, reqwest_err, to_value};
use reqwest::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A Grafana API client that resolves credentials from the workflow's secret store.
///
/// Wraps a [`reqwest::Client`] with a base URL and bearer token. All operation
/// structs in this crate accept a `&GrafanaClient` to make HTTP calls.
///
/// # Construction
///
/// - [`from_context`](GrafanaClient::from_context) reads `grafana_token` and
///   optionally `grafana_url` from the secret store.
/// - [`from_context_with_url`](GrafanaClient::from_context_with_url) reads
///   `grafana_token` and uses an explicit base URL.
/// - [`new`](GrafanaClient::new) accepts explicit token and URL.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let grafana = GrafanaClient::from_context(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct GrafanaClient {
    inner: HttpApiClient,
}

impl GrafanaClient {
    /// Build a client from an [`OperationContext`].
    ///
    /// Reads the `grafana_token` secret for authentication and the optional
    /// `grafana_url` secret for the base URL (defaults to `http://localhost:3000`).
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the token is missing, or
    /// [`OperationError::Http`] if the HTTP client cannot be built.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_grafana::GrafanaClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let grafana = GrafanaClient::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let token =
            ctx.secrets()
                .get("grafana_token")
                .await?
                .ok_or_else(|| OperationError::Secret {
                    message: "grafana_token secret not found".to_string(),
                })?;

        let url = match ctx.secrets().get("grafana_url").await? {
            Some(s) => s.value.clone(),
            None => "http://localhost:3000".to_string(),
        };

        Self::new(&token.value, &url)
    }

    /// Build a client from an [`OperationContext`] with an explicit base URL.
    ///
    /// Reads the `grafana_token` secret for authentication.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the token is missing, or
    /// [`OperationError::Http`] if the HTTP client cannot be built.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_grafana::GrafanaClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let grafana = GrafanaClient::from_context_with_url(&ctx, "https://grafana.example.com").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context_with_url(
        ctx: &OperationContext,
        base_url: &str,
    ) -> Result<Self, OperationError> {
        let token =
            ctx.secrets()
                .get("grafana_token")
                .await?
                .ok_or_else(|| OperationError::Secret {
                    message: "grafana_token secret not found".to_string(),
                })?;

        Self::new(&token.value, base_url)
    }

    /// Build a client with an explicit token and base URL.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the HTTP client cannot be built.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_grafana::GrafanaClient;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let grafana = GrafanaClient::new("glsa_xxxx", "https://grafana.example.com")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(token: &str, base_url: &str) -> Result<Self, OperationError> {
        if token.trim().is_empty() {
            return Err(OperationError::Secret {
                message: "grafana token must not be empty".to_string(),
            });
        }

        let http = Client::builder()
            .build()
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("failed to build HTTP client: {e}"),
            })?;

        let inner = HttpApiClient::new(base_url, http).with_bearer_token(token);
        Ok(Self { inner })
    }

    /// The base URL (without trailing slash).
    pub fn base_url(&self) -> &str {
        self.inner.base_url()
    }

    /// Build a full API URL by appending `path` to the base URL.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_grafana::GrafanaClient;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let client = GrafanaClient::new("token", "https://grafana.example.com")?;
    /// assert_eq!(client.url("/api/dashboards/uid/abc"), "https://grafana.example.com/api/dashboards/uid/abc");
    /// # Ok(())
    /// # }
    /// ```
    pub fn url(&self, path: &str) -> String {
        self.inner.url(path)
    }

    /// Build an authenticated GET [`RequestBuilder`] for the given path.
    ///
    /// Use this when you need to add query parameters or other customization
    /// before sending. For simple requests, prefer [`get_json`](Self::get_json).
    pub(crate) fn get_request(&self, path: &str) -> reqwest::RequestBuilder {
        self.inner.get(path)
    }

    /// Send a request and deserialize the JSON response.
    async fn send_json<T: DeserializeOwned>(
        req: reqwest::RequestBuilder,
    ) -> Result<T, OperationError> {
        let resp = req.send().await.map_err(reqwest_err)?;
        check_response_json(resp).await
    }

    /// Send a GET request with query parameters and deserialize the JSON response.
    pub(crate) async fn get_json_with_query<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.get(path).query(query)).await
    }

    /// Send a GET request and deserialize the JSON response.
    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.get(path)).await
    }

    /// Send a POST request with a JSON body and deserialize the response.
    pub(crate) async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.post(path).json(body)).await
    }

    /// Send a PUT request with a JSON body and deserialize the response.
    pub(crate) async fn put_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.put(path).json(body)).await
    }

    /// Send a PATCH request with a JSON body and deserialize the response.
    pub(crate) async fn patch_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.patch(path).json(body)).await
    }

    /// Send a DELETE request and deserialize the JSON response.
    pub(crate) async fn delete_json<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.delete(path)).await
    }

    /// Serialize a value to [`serde_json::Value`].
    pub(crate) fn to_value<T: Serialize>(val: &T) -> Result<Value, OperationError> {
        to_value(val)
    }
}

impl fmt::Debug for GrafanaClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrafanaClient")
            .field("base_url", &self.inner.base_url())
            .field("auth", self.inner.auth())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    use super::*;

    #[tokio::test]
    async fn from_context_fails_when_token_missing() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = GrafanaClient::from_context(&ctx).await.unwrap_err();
        assert!(
            err.to_string().contains("grafana_token"),
            "expected grafana_token error, got: {err}"
        );
    }

    #[test]
    fn normalizes_base_url_trailing_slash() {
        let client = GrafanaClient::new("tok", "https://grafana.example.com/").unwrap();
        assert_eq!(client.base_url(), "https://grafana.example.com");
    }

    #[test]
    fn normalizes_base_url_no_trailing_slash() {
        let client = GrafanaClient::new("tok", "https://grafana.example.com").unwrap();
        assert_eq!(client.base_url(), "https://grafana.example.com");
    }

    #[test]
    fn debug_does_not_leak_token() {
        let client =
            GrafanaClient::new("super-secret-token", "https://grafana.example.com").unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains("super-secret-token"));
        assert!(debug.contains("redacted"));
        assert!(debug.contains("GrafanaClient"));
    }

    #[test]
    fn url_builds_correct_path() {
        let client = GrafanaClient::new("tok", "https://grafana.example.com").unwrap();
        assert_eq!(
            client.url("/api/dashboards/uid/abc"),
            "https://grafana.example.com/api/dashboards/uid/abc"
        );
    }
}
