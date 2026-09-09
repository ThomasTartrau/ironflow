//! Miscellaneous operations.
//!
//! Provides short URLs, frontend settings, and auth renewal
//! via various Grafana API endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;

/// Short URL creation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortUrlOutput {
    /// Short URL UID.
    pub uid: Option<String>,
    /// Full short URL.
    pub url: Option<String>,
}

/// Create a short URL.
///
/// Sends a `POST /api/short-urls` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::other::CreateShortUrl;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"path": "/d/abc123/my-dashboard"});
/// let op = CreateShortUrl::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct CreateShortUrl {
    client: GrafanaClient,
    body: Value,
}

impl CreateShortUrl {
    /// Create a short-URL operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            client: client.clone(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<ShortUrlOutput, OperationError> {
        self.client.post_json("/api/short-urls", &self.body).await
    }
}

#[async_trait]
impl Operation for CreateShortUrl {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(self.body.clone())
    }
}

impl TypedOperation for CreateShortUrl {
    type Output = ShortUrlOutput;
}

/// Get frontend settings.
///
/// Sends a `GET /api/frontend/settings` request. The response shape is
/// dynamic and not fully typed.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::other::GetFrontendSettings;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = GetFrontendSettings::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct GetFrontendSettings {
    client: GrafanaClient,
}

impl GetFrontendSettings {
    /// Create a get-frontend-settings operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            client: client.clone(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client.get_json("/api/frontend/settings").await
    }
}

#[async_trait]
impl Operation for GetFrontendSettings {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "endpoint": "frontend/settings" }))
    }
}

/// Renew authentication session.
///
/// Sends a `GET /api/auth/renew` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::other::RenewAuth;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = RenewAuth::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct RenewAuth {
    client: GrafanaClient,
}

impl RenewAuth {
    /// Create a renew-auth operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            client: client.clone(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client.get_json("/api/auth/renew").await
    }
}

#[async_trait]
impl Operation for RenewAuth {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "endpoint": "auth/renew" }))
    }
}
