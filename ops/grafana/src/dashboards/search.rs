//! Dashboard search operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{get, to_value};

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
    url: String,
    token: String,
    http: reqwest::Client,
    query: Option<String>,
    tag: Option<String>,
}

impl DashboardSearch {
    /// Create a search operation.
    ///
    /// Both `query` and `tag` are optional filters.
    pub fn new(client: &GrafanaClient, query: Option<&str>, tag: Option<&str>) -> Self {
        let base = client.url("/api/search");
        let mut url = reqwest::Url::parse(&base).expect("base URL is always valid");
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("type", "dash-db");
            if let Some(q) = query {
                pairs.append_pair("query", q);
            }
            if let Some(t) = tag {
                pairs.append_pair("tag", t);
            }
        }
        Self {
            url: url.to_string(),
            token: client.token().to_string(),
            http: client.http().clone(),
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
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for DashboardSearch {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "query": self.query, "tag": self.tag }))
    }
}

impl TypedOperation for DashboardSearch {
    type Output = Vec<DashboardSearchHit>;
}
