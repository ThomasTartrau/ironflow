//! LogQL query formatting operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::check_response;

/// Format a LogQL expression.
///
/// Calls `GET /loki/api/v1/format_query` with the given query. Returns
/// the pretty-printed version of the query.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, format::FormatQuery};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = FormatQuery::new(loki, r#"{job="varlogs"} |= "error""#);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct FormatQuery {
    client: LokiClient,
    query: String,
}

impl FormatQuery {
    /// Create a new format query operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, format::FormatQuery};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = FormatQuery::new(loki, r#"{job="varlogs"}"#);
    /// ```
    pub fn new(client: LokiClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for FormatQuery {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "format_query",
            "query": self.query,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/loki/api/v1/format_query")
            .query(&[("query", &self.query)])
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("format query request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
