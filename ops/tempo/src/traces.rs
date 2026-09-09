//! Trace retrieval and search operations.
//!
//! These operations query traces from Tempo by ID or via TraceQL search.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::TempoClient;
use crate::error::{check_response, parse_json_body, send_request, validate_path_segment};

/// Retrieve a trace by its ID.
///
/// Calls `GET /api/traces/{traceID}`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, traces::GetTrace};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetTrace::new(tempo, "abc123def456");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetTrace {
    client: TempoClient,
    trace_id: String,
    start: Option<String>,
    end: Option<String>,
}

impl GetTrace {
    /// Create a new trace retrieval operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, traces::GetTrace};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetTrace::new(tempo, "abc123def456");
    /// ```
    pub fn new(client: TempoClient, trace_id: &str) -> Self {
        Self {
            client,
            trace_id: trace_id.to_owned(),
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
impl Operation for GetTrace {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_trace",
            "trace_id": self.trace_id,
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the trace ID is invalid, or
    /// [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.trace_id, "trace_id")?;

        let mut req = self.client.get(&format!("/api/traces/{}", self.trace_id));
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = send_request(req, "get trace").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Retrieve a trace by its ID using the v2 endpoint.
///
/// Calls `GET /api/v2/traces/{traceID}`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, traces::GetTraceV2};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetTraceV2::new(tempo, "abc123def456");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetTraceV2 {
    client: TempoClient,
    trace_id: String,
    start: Option<String>,
    end: Option<String>,
}

impl GetTraceV2 {
    /// Create a new v2 trace retrieval operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, traces::GetTraceV2};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetTraceV2::new(tempo, "abc123def456");
    /// ```
    pub fn new(client: TempoClient, trace_id: &str) -> Self {
        Self {
            client,
            trace_id: trace_id.to_owned(),
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
impl Operation for GetTraceV2 {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_trace_v2",
            "trace_id": self.trace_id,
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the trace ID is invalid, or
    /// [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.trace_id, "trace_id")?;

        let mut req = self
            .client
            .get(&format!("/api/v2/traces/{}", self.trace_id));
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = send_request(req, "get trace v2").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Search for traces using TraceQL.
///
/// Calls `GET /api/search` with query parameters.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, traces::SearchTraces};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = SearchTraces::new(tempo, r#"{ span.http.status_code = 500 }"#);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct SearchTraces {
    client: TempoClient,
    query: String,
    limit: Option<u64>,
    start: Option<String>,
    end: Option<String>,
    min_duration: Option<String>,
    max_duration: Option<String>,
    spss: Option<u64>,
}

impl SearchTraces {
    /// Create a new trace search operation.
    ///
    /// The `query` parameter is a TraceQL expression.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, traces::SearchTraces};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = SearchTraces::new(tempo, r#"{ span.http.status_code = 500 }"#);
    /// ```
    pub fn new(client: TempoClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            limit: None,
            start: None,
            end: None,
            min_duration: None,
            max_duration: None,
            spss: None,
        }
    }

    /// Set the maximum number of traces to return.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
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

    /// Set the minimum span duration (e.g. `"100ms"`).
    #[must_use]
    pub fn min_duration(mut self, min_duration: &str) -> Self {
        self.min_duration = Some(min_duration.to_owned());
        self
    }

    /// Set the maximum span duration (e.g. `"5s"`).
    #[must_use]
    pub fn max_duration(mut self, max_duration: &str) -> Self {
        self.max_duration = Some(max_duration.to_owned());
        self
    }

    /// Set the number of spans per span set (spss).
    #[must_use]
    pub fn spss(mut self, spss: u64) -> Self {
        self.spss = Some(spss);
        self
    }
}

#[async_trait]
impl Operation for SearchTraces {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "search_traces",
            "query": self.query,
            "limit": self.limit,
            "start": self.start,
            "end": self.end,
            "minDuration": self.min_duration,
            "maxDuration": self.max_duration,
            "spss": self.spss,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self.client.get("/api/search").query(&[("q", &self.query)]);
        if let Some(l) = self.limit {
            req = req.query(&[("limit", &l.to_string())]);
        }
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }
        if let Some(ref m) = self.min_duration {
            req = req.query(&[("minDuration", m)]);
        }
        if let Some(ref m) = self.max_duration {
            req = req.query(&[("maxDuration", m)]);
        }
        if let Some(s) = self.spss {
            req = req.query(&[("spss", &s.to_string())]);
        }

        let response = send_request(req, "search traces").await?;
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

    #[tokio::test]
    async fn get_trace_sends_correct_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/traces/abc123"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"batches":[]}"#))
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let op = GetTrace::new(tempo, "abc123");
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = op.execute(&ctx).await.unwrap();
        assert_eq!(result["batches"], json!([]));
    }

    #[tokio::test]
    async fn get_trace_rejects_invalid_trace_id() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());
        let op = GetTrace::new(tempo, "../admin");
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = op.execute(&ctx).await.unwrap_err();
        assert!(
            matches!(err, OperationError::External { .. }),
            "expected External error for invalid trace ID, got: {err}"
        );
    }

    #[tokio::test]
    async fn search_traces_passes_query_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/search"))
            .and(query_param("q", "{ span.http.status_code = 500 }"))
            .and(query_param("limit", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"traces":[]}"#))
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let op = SearchTraces::new(tempo, "{ span.http.status_code = 500 }").limit(10);
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = op.execute(&ctx).await.unwrap();
        assert_eq!(result["traces"], json!([]));
    }

    #[test]
    fn get_trace_kind_is_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());
        let op = GetTrace::new(tempo, "abc");
        assert_eq!(op.kind(), "tempo");
    }

    #[test]
    fn get_trace_v2_kind_is_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());
        let op = GetTraceV2::new(tempo, "abc");
        assert_eq!(op.kind(), "tempo");
    }

    #[test]
    fn search_traces_kind_is_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());
        let op = SearchTraces::new(tempo, "{}");
        assert_eq!(op.kind(), "tempo");
    }
}
