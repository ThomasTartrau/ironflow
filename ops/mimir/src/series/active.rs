//! Active series cardinality operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Retrieve the list of active series (Mimir-specific).
///
/// Calls `GET /prometheus/api/v1/cardinality/active_series` with a selector.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, series::GetActiveSeries};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetActiveSeries::new(mimir, "up");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetActiveSeries {
    client: MimirClient,
    selector: String,
}

impl GetActiveSeries {
    /// Create a new active series query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, series::GetActiveSeries};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetActiveSeries::new(mimir, "up");
    /// ```
    pub fn new(client: MimirClient, selector: &str) -> Self {
        Self {
            client,
            selector: selector.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for GetActiveSeries {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_active_series",
            "selector": self.selector,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let req = self
            .client
            .get("/prometheus/api/v1/cardinality/active_series")
            .query(&[("selector", &self.selector)]);

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get active series request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
