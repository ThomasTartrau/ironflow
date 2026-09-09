//! Log query operations: instant queries, range queries, and tail (streaming).
//!
//! [`QueryInstant`] and [`QueryRange`] implement [`Operation`] and return the
//! Loki API response as a [`Value`]. [`TailLogs`] is a WebSocket streaming
//! operation that does not implement `Operation` -- access it directly via
//! [`LokiClient::http_client`](crate::LokiClient::http_client) and the
//! Loki tail endpoint.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde::Serialize;
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::check_response;

/// Execute an instant query against Loki at a single point in time.
///
/// Calls `GET /loki/api/v1/query` with the given LogQL expression and
/// optional evaluation timestamp.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, query::QueryInstant};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = QueryInstant::new(loki, r#"{job="varlogs"}"#);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct QueryInstant {
    client: LokiClient,
    query: String,
    time: Option<String>,
    limit: Option<u64>,
    direction: Option<String>,
}

impl QueryInstant {
    /// Create a new instant query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, query::QueryInstant};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = QueryInstant::new(loki, r#"{job="varlogs"}"#);
    /// ```
    pub fn new(client: LokiClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            time: None,
            limit: None,
            direction: None,
        }
    }

    /// Set the evaluation timestamp (RFC3339 or Unix epoch).
    #[must_use]
    pub fn time(mut self, time: &str) -> Self {
        self.time = Some(time.to_owned());
        self
    }

    /// Set the maximum number of entries to return.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set the log entry order: `"forward"` or `"backward"`.
    #[must_use]
    pub fn direction(mut self, direction: &str) -> Self {
        self.direction = Some(direction.to_owned());
        self
    }
}

#[async_trait]
impl Operation for QueryInstant {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "query_instant",
            "query": self.query,
            "time": self.time,
            "limit": self.limit,
            "direction": self.direction,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self
            .client
            .get("/loki/api/v1/query")
            .query(&[("query", &self.query)]);
        if let Some(ref t) = self.time {
            req = req.query(&[("time", t)]);
        }
        if let Some(l) = self.limit {
            req = req.query(&[("limit", &l.to_string())]);
        }
        if let Some(ref d) = self.direction {
            req = req.query(&[("direction", d)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("query instant request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Execute a range query against Loki over a time window.
///
/// Calls `GET /loki/api/v1/query_range` with the given LogQL expression,
/// start/end timestamps, and optional step and limit.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, query::QueryRange};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = QueryRange::new(loki, r#"{job="varlogs"}"#, "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct QueryRange {
    client: LokiClient,
    query: String,
    start: String,
    end: String,
    step: Option<String>,
    limit: Option<u64>,
    direction: Option<String>,
}

impl QueryRange {
    /// Create a new range query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, query::QueryRange};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = QueryRange::new(loki, r#"{job="varlogs"}"#, "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
    /// ```
    pub fn new(client: LokiClient, query: &str, start: &str, end: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
            step: None,
            limit: None,
            direction: None,
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

    /// Set the log entry order: `"forward"` or `"backward"`.
    #[must_use]
    pub fn direction(mut self, direction: &str) -> Self {
        self.direction = Some(direction.to_owned());
        self
    }
}

#[async_trait]
impl Operation for QueryRange {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "query_range",
            "query": self.query,
            "start": self.start,
            "end": self.end,
            "step": self.step,
            "limit": self.limit,
            "direction": self.direction,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self.client.get("/loki/api/v1/query_range").query(&[
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
        if let Some(ref d) = self.direction {
            req = req.query(&[("direction", d)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("query range request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Tail (stream) logs matching a LogQL query via WebSocket.
///
/// This is a streaming operation that does **not** implement [`Operation`],
/// following the same pattern as Kubernetes watch/exec/attach operations in
/// `ironflow_ops_k8s`. Access it directly via the Loki WebSocket endpoint
/// at `/loki/api/v1/tail`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::LokiClient;
/// use reqwest::Client;
///
/// # async fn example() {
/// let loki = LokiClient::new("http://loki:3100", Client::new());
/// // Connect to ws://<base>/loki/api/v1/tail?query={job="varlogs"}
/// // using a WebSocket client of your choice.
/// let tail_url = format!("{}/loki/api/v1/tail", loki.base_url());
/// # }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct TailLogs {
    /// The LogQL query to stream.
    pub query: String,
    /// Maximum number of seconds to delay log entries.
    pub delay_for: Option<u64>,
    /// Maximum number of entries to return per request.
    pub limit: Option<u64>,
    /// Start timestamp (RFC3339 or Unix epoch).
    pub start: Option<String>,
}
