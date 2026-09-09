//! Series search operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Retrieve time series matching a set of label matchers.
///
/// Calls `GET /prometheus/api/v1/series` with one or more `match[]` parameters.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, series::GetSeries};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetSeries::new(mimir, vec!["up".into()]);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetSeries {
    client: MimirClient,
    matchers: Vec<String>,
    start: Option<String>,
    end: Option<String>,
}

impl GetSeries {
    /// Create a new series query with the given label matchers.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, series::GetSeries};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetSeries::new(mimir, vec!["up".into()]);
    /// ```
    pub fn new(client: MimirClient, matchers: Vec<String>) -> Self {
        Self {
            client,
            matchers,
            start: None,
            end: None,
        }
    }

    /// Restrict results to series seen after this timestamp.
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Restrict results to series seen before this timestamp.
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }
}

#[async_trait]
impl Operation for GetSeries {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_series",
            "matchers": self.matchers,
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let match_pairs: Vec<(&str, &String)> =
            self.matchers.iter().map(|m| ("match[]", m)).collect();
        let mut req = self
            .client
            .get("/prometheus/api/v1/series")
            .query(&match_pairs);
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get series request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
