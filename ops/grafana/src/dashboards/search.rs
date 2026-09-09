//! Dashboard search operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use ironflow_ops_common::helpers::{check_response_json, reqwest_err};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::DashboardSearchHit;

/// Search dashboards.
///
/// Sends a `GET /api/search?type=dash-db` request with optional query parameters.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardSearch;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DashboardSearch::new(&grafana, Some("production"), None);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardSearch {
    client: GrafanaClient,
    query: Option<String>,
    tag: Option<String>,
}

impl DashboardSearch {
    /// Create a search operation.
    ///
    /// Both `query` and `tag` are optional filters.
    pub fn new(client: &GrafanaClient, query: Option<&str>, tag: Option<&str>) -> Self {
        Self {
            client: client.clone(),
            query: query.map(String::from),
            tag: tag.map(String::from),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<DashboardSearchHit>, OperationError> {
        let mut req = self
            .client
            .get_request("/api/search")
            .query(&[("type", "dash-db")]);
        if let Some(ref q) = self.query {
            req = req.query(&[("query", q.as_str())]);
        }
        if let Some(ref t) = self.tag {
            req = req.query(&[("tag", t.as_str())]);
        }
        let resp = req.send().await.map_err(reqwest_err)?;
        check_response_json(resp).await
    }
}

#[async_trait]
impl Operation for DashboardSearch {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "query": self.query, "tag": self.tag }))
    }
}

impl TypedOperation for DashboardSearch {
    type Output = Vec<DashboardSearchHit>;
}
