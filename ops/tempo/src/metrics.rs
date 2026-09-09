//! TraceQL metrics query operations.
//!
//! These operations query Tempo's metrics-generator endpoints for
//! range and instant metric queries derived from trace data.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::TempoClient;
use crate::error::{check_response, parse_json_body, send_request};

/// Execute a range metrics query.
///
/// Calls `GET /api/metrics/query_range` with the given TraceQL metrics query
/// and time range parameters.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, metrics::QueryMetricsRange};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = QueryMetricsRange::new(tempo, "{ } | rate()");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct QueryMetricsRange {
    client: TempoClient,
    query: String,
    start: Option<String>,
    end: Option<String>,
    step: Option<String>,
}

impl QueryMetricsRange {
    /// Create a new range metrics query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, metrics::QueryMetricsRange};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = QueryMetricsRange::new(tempo, "{ } | rate()");
    /// ```
    pub fn new(client: TempoClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: None,
            end: None,
            step: None,
        }
    }

    /// Set the start time (Unix epoch seconds).
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Set the end time (Unix epoch seconds).
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }

    /// Set the query resolution step (e.g. `"15s"`).
    #[must_use]
    pub fn step(mut self, step: &str) -> Self {
        self.step = Some(step.to_owned());
        self
    }
}

#[async_trait]
impl Operation for QueryMetricsRange {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "query_metrics_range",
            "query": self.query,
            "start": self.start,
            "end": self.end,
            "step": self.step,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self
            .client
            .get("/api/metrics/query_range")
            .query(&[("q", &self.query)]);
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }
        if let Some(ref s) = self.step {
            req = req.query(&[("step", s)]);
        }

        let response = send_request(req, "query metrics range").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Execute an instant metrics query.
///
/// Calls `GET /api/metrics/query` with the given TraceQL metrics query.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, metrics::QueryMetricsInstant};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = QueryMetricsInstant::new(tempo, "{ } | rate()");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct QueryMetricsInstant {
    client: TempoClient,
    query: String,
    start: Option<String>,
    end: Option<String>,
}

impl QueryMetricsInstant {
    /// Create a new instant metrics query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, metrics::QueryMetricsInstant};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = QueryMetricsInstant::new(tempo, "{ } | rate()");
    /// ```
    pub fn new(client: TempoClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: None,
            end: None,
        }
    }

    /// Set the start time (Unix epoch seconds).
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Set the end time (Unix epoch seconds).
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }
}

#[async_trait]
impl Operation for QueryMetricsInstant {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "query_metrics_instant",
            "query": self.query,
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self
            .client
            .get("/api/metrics/query")
            .query(&[("q", &self.query)]);
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = send_request(req, "query metrics instant").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
    use reqwest::Client;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::TempoClient;

    #[test]
    fn all_metrics_ops_return_kind_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());

        assert_eq!(QueryMetricsRange::new(tempo.clone(), "{}").kind(), "tempo");
        assert_eq!(QueryMetricsInstant::new(tempo, "{}").kind(), "tempo");
    }

    #[tokio::test]
    async fn query_metrics_range_sends_all_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/metrics/query_range"))
            .and(query_param("q", "{ } | rate()"))
            .and(query_param("start", "1000"))
            .and(query_param("end", "2000"))
            .and(query_param("step", "15s"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":{"result":[]}}"#))
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = QueryMetricsRange::new(tempo, "{ } | rate()")
            .start("1000")
            .end("2000")
            .step("15s")
            .execute(&ctx)
            .await
            .unwrap();
        assert_eq!(result["data"]["result"], json!([]));
    }

    #[tokio::test]
    async fn query_metrics_instant_sends_query_param() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/metrics/query"))
            .and(query_param("q", "{ } | count_over_time()"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":{"result":[]}}"#))
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = QueryMetricsInstant::new(tempo, "{ } | count_over_time()")
            .execute(&ctx)
            .await
            .unwrap();
        assert_eq!(result["data"]["result"], json!([]));
    }
}
