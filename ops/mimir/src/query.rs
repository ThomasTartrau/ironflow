//! PromQL query operations: instant queries, range queries, exemplars, and formatting.
//!
//! [`QueryInstant`] and [`QueryRange`] execute PromQL expressions against the
//! Mimir query frontend. [`QueryExemplars`] fetches exemplars for a given
//! expression. [`FormatQuery`] pretty-prints a PromQL expression.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Execute an instant PromQL query at a single point in time.
///
/// Calls `GET /prometheus/api/v1/query` with the given expression and
/// optional evaluation timestamp.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, query::QueryInstant};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = QueryInstant::new(mimir, "up");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct QueryInstant {
    client: MimirClient,
    query: String,
    time: Option<String>,
    timeout: Option<String>,
}

impl QueryInstant {
    /// Create a new instant query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, query::QueryInstant};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = QueryInstant::new(mimir, "up");
    /// ```
    pub fn new(client: MimirClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            time: None,
            timeout: None,
        }
    }

    /// Set the evaluation timestamp (RFC3339 or Unix epoch).
    #[must_use]
    pub fn time(mut self, time: &str) -> Self {
        self.time = Some(time.to_owned());
        self
    }

    /// Set the evaluation timeout (e.g. `"30s"`).
    #[must_use]
    pub fn timeout(mut self, timeout: &str) -> Self {
        self.timeout = Some(timeout.to_owned());
        self
    }
}

#[async_trait]
impl Operation for QueryInstant {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "query_instant",
            "query": self.query,
            "time": self.time,
            "timeout": self.timeout,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self
            .client
            .get("/prometheus/api/v1/query")
            .query(&[("query", &self.query)]);
        if let Some(ref t) = self.time {
            req = req.query(&[("time", t)]);
        }
        if let Some(ref t) = self.timeout {
            req = req.query(&[("timeout", t)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("query instant request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Execute a range PromQL query over a time window.
///
/// Calls `GET /prometheus/api/v1/query_range` with the given expression,
/// start/end timestamps, and optional step.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, query::QueryRange};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = QueryRange::new(mimir, "up", "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct QueryRange {
    client: MimirClient,
    query: String,
    start: String,
    end: String,
    step: Option<String>,
    timeout: Option<String>,
}

impl QueryRange {
    /// Create a new range query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, query::QueryRange};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = QueryRange::new(mimir, "up", "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
    /// ```
    pub fn new(client: MimirClient, query: &str, start: &str, end: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
            step: None,
            timeout: None,
        }
    }

    /// Set the query resolution step (e.g. `"5m"`, `"1h"`).
    #[must_use]
    pub fn step(mut self, step: &str) -> Self {
        self.step = Some(step.to_owned());
        self
    }

    /// Set the evaluation timeout (e.g. `"30s"`).
    #[must_use]
    pub fn timeout(mut self, timeout: &str) -> Self {
        self.timeout = Some(timeout.to_owned());
        self
    }
}

#[async_trait]
impl Operation for QueryRange {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "query_range",
            "query": self.query,
            "start": self.start,
            "end": self.end,
            "step": self.step,
            "timeout": self.timeout,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self.client.get("/prometheus/api/v1/query_range").query(&[
            ("query", &self.query),
            ("start", &self.start),
            ("end", &self.end),
        ]);
        if let Some(ref s) = self.step {
            req = req.query(&[("step", s)]);
        }
        if let Some(ref t) = self.timeout {
            req = req.query(&[("timeout", t)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("query range request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Query exemplars for a given PromQL expression.
///
/// Calls `GET /prometheus/api/v1/query_exemplars` with the given expression
/// and time range.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, query::QueryExemplars};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = QueryExemplars::new(mimir, "http_requests_total", "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct QueryExemplars {
    client: MimirClient,
    query: String,
    start: String,
    end: String,
}

impl QueryExemplars {
    /// Create a new exemplars query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, query::QueryExemplars};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = QueryExemplars::new(mimir, "http_requests_total", "s", "e");
    /// ```
    pub fn new(client: MimirClient, query: &str, start: &str, end: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for QueryExemplars {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "query_exemplars",
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
        let req = self
            .client
            .get("/prometheus/api/v1/query_exemplars")
            .query(&[
                ("query", &self.query),
                ("start", &self.start),
                ("end", &self.end),
            ]);

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("query exemplars request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Format (pretty-print) a PromQL expression.
///
/// Calls `GET /prometheus/api/v1/format_query` with the given expression.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, query::FormatQuery};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = FormatQuery::new(mimir, "up == 1");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct FormatQuery {
    client: MimirClient,
    query: String,
}

impl FormatQuery {
    /// Create a new format query operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, query::FormatQuery};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = FormatQuery::new(mimir, "up == 1");
    /// ```
    pub fn new(client: MimirClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for FormatQuery {
    fn kind(&self) -> &str {
        "mimir"
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
        let req = self
            .client
            .get("/prometheus/api/v1/format_query")
            .query(&[("query", &self.query)]);

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("format query request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
