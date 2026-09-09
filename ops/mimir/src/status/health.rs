//! Health, metrics, and services operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Check if Mimir is ready to accept requests.
///
/// Calls `GET /ready`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, status::GetReady};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetReady::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetReady {
    client: MimirClient,
}

impl GetReady {
    /// Create a new readiness check operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, status::GetReady};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetReady::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetReady {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_ready"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or Mimir is not ready.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/ready")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("readiness check failed: {e}"),
                })?;

        let status = response.status().as_u16();
        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ready": true, "status": status, "body": text.as_ref()}))
    }
}

/// Retrieve Prometheus metrics from Mimir.
///
/// Calls `GET /metrics`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, status::GetMetrics};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetMetrics::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetMetrics {
    client: MimirClient,
}

impl GetMetrics {
    /// Create a new metrics query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, status::GetMetrics};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetMetrics::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetMetrics {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_metrics"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/metrics")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("get metrics request failed: {e}"),
                })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"metrics": text.as_ref()}))
    }
}

/// List running services and their status.
///
/// Calls `GET /services`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, status::GetServices};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetServices::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetServices {
    client: MimirClient,
}

impl GetServices {
    /// Create a new services query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, status::GetServices};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetServices::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetServices {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_services"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/services")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("get services request failed: {e}"),
                })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"services": text.as_ref()}))
    }
}
