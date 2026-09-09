//! Metric metadata operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Retrieve metric metadata.
///
/// Calls `GET /prometheus/api/v1/metadata` with an optional metric name filter.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, series::GetMetadata};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetMetadata::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetMetadata {
    client: MimirClient,
    metric: Option<String>,
    limit: Option<u64>,
}

impl GetMetadata {
    /// Create a new metadata query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, series::GetMetadata};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetMetadata::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self {
            client,
            metric: None,
            limit: None,
        }
    }

    /// Filter by metric name.
    #[must_use]
    pub fn metric(mut self, metric: &str) -> Self {
        self.metric = Some(metric.to_owned());
        self
    }

    /// Limit the number of results.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }
}

#[async_trait]
impl Operation for GetMetadata {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_metadata",
            "metric": self.metric,
            "limit": self.limit,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self.client.get("/prometheus/api/v1/metadata");
        if let Some(ref m) = self.metric {
            req = req.query(&[("metric", m)]);
        }
        if let Some(l) = self.limit {
            req = req.query(&[("limit", &l.to_string())]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get metadata request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
