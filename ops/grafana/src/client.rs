//! [`GrafanaClient`] built from an [`OperationContext`]'s secret store.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use reqwest::Client;

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
pub struct GrafanaClient {
    http: Client,
    base_url: String,
    token: String,
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

        let base_url = base_url.trim_end_matches('/').to_string();

        Ok(Self {
            http,
            base_url,
            token: token.to_string(),
        })
    }

    /// The underlying [`reqwest::Client`].
    pub(crate) fn http(&self) -> &Client {
        &self.http
    }

    /// The base URL (without trailing slash).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The bearer token.
    pub(crate) fn token(&self) -> &str {
        &self.token
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
        format!("{}{path}", self.base_url)
    }
}

impl fmt::Debug for GrafanaClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrafanaClient")
            .field("base_url", &self.base_url)
            .field("token", &"[REDACTED]")
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
        assert!(debug.contains("REDACTED"));
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
