//! Index statistics and volume operations.
//!
//! These operations query Loki's index for statistics about stored data,
//! useful for capacity planning and cost analysis.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::check_response;

/// Retrieve index statistics.
///
/// Calls `GET /loki/api/v1/index/stats` with an optional query and time range.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, index::GetIndexStats};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetIndexStats::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetIndexStats {
    client: LokiClient,
    query: Option<String>,
    start: Option<String>,
    end: Option<String>,
}

impl GetIndexStats {
    /// Create a new index stats query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, index::GetIndexStats};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetIndexStats::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self {
            client,
            query: None,
            start: None,
            end: None,
        }
    }

    /// Filter stats to a specific LogQL query.
    #[must_use]
    pub fn query(mut self, query: &str) -> Self {
        self.query = Some(query.to_owned());
        self
    }

    /// Restrict results to data after this timestamp.
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Restrict results to data before this timestamp.
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }
}

#[async_trait]
impl Operation for GetIndexStats {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_index_stats",
            "query": self.query,
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self.client.get("/loki/api/v1/index/stats");
        if let Some(ref q) = self.query {
            req = req.query(&[("query", q)]);
        }
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get index stats request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve index volume for label combinations.
///
/// Calls `GET /loki/api/v1/index/volume` with an optional query and time range.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, index::GetIndexVolume};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetIndexVolume::new(loki, r#"{job="varlogs"}"#);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetIndexVolume {
    client: LokiClient,
    query: String,
    start: Option<String>,
    end: Option<String>,
    limit: Option<u64>,
}

impl GetIndexVolume {
    /// Create a new index volume query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, index::GetIndexVolume};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetIndexVolume::new(loki, r#"{job="varlogs"}"#);
    /// ```
    pub fn new(client: LokiClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: None,
            end: None,
            limit: None,
        }
    }

    /// Restrict results to data after this timestamp.
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Restrict results to data before this timestamp.
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }

    /// Set the maximum number of entries to return.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }
}

#[async_trait]
impl Operation for GetIndexVolume {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_index_volume",
            "query": self.query,
            "start": self.start,
            "end": self.end,
            "limit": self.limit,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self
            .client
            .get("/loki/api/v1/index/volume")
            .query(&[("query", &self.query)]);
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }
        if let Some(l) = self.limit {
            req = req.query(&[("limit", &l.to_string())]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get index volume request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve index volume over a time range.
///
/// Calls `GET /loki/api/v1/index/volume_range` with query, time range, and step.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, index::GetIndexVolumeRange};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetIndexVolumeRange::new(loki, r#"{job="varlogs"}"#, "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetIndexVolumeRange {
    client: LokiClient,
    query: String,
    start: String,
    end: String,
    step: Option<String>,
    limit: Option<u64>,
}

impl GetIndexVolumeRange {
    /// Create a new index volume range query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, index::GetIndexVolumeRange};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetIndexVolumeRange::new(loki, r#"{job="varlogs"}"#, "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
    /// ```
    pub fn new(client: LokiClient, query: &str, start: &str, end: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
            step: None,
            limit: None,
        }
    }

    /// Set the query resolution step (e.g. `"5m"`, `"1h"`).
    #[must_use]
    pub fn step(mut self, step: &str) -> Self {
        self.step = Some(step.to_owned());
        self
    }

    /// Set the maximum number of entries to return.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }
}

#[async_trait]
impl Operation for GetIndexVolumeRange {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_index_volume_range",
            "query": self.query,
            "start": self.start,
            "end": self.end,
            "step": self.step,
            "limit": self.limit,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self.client.get("/loki/api/v1/index/volume_range").query(&[
            ("query", &self.query),
            ("start", &self.start),
            ("end", &self.end),
        ]);
        if let Some(ref s) = self.step {
            req = req.query(&[("step", s)]);
        }
        if let Some(l) = self.limit {
            req = req.query(&[("limit", &l.to_string())]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get index volume range request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
