//! Pattern and field detection operations.
//!
//! These operations use Loki's built-in pattern and field detection to
//! discover log structure without writing explicit parsers.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::{check_response, validate_path_segment};

/// Detect common patterns in log lines.
///
/// Calls `GET /loki/api/v1/patterns` with the given query and time range.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, patterns::DetectPatterns};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = DetectPatterns::new(loki, r#"{job="varlogs"}"#);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DetectPatterns {
    client: LokiClient,
    query: String,
    start: Option<String>,
    end: Option<String>,
}

impl DetectPatterns {
    /// Create a new pattern detection operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, patterns::DetectPatterns};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = DetectPatterns::new(loki, r#"{job="varlogs"}"#);
    /// ```
    pub fn new(client: LokiClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: None,
            end: None,
        }
    }

    /// Restrict detection to data after this timestamp.
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Restrict detection to data before this timestamp.
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }
}

#[async_trait]
impl Operation for DetectPatterns {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "detect_patterns",
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
        let mut req = self
            .client
            .get("/loki/api/v1/patterns")
            .query(&[("query", &self.query)]);
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("detect patterns request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Detect fields in log lines.
///
/// Calls `GET /loki/api/v1/detected_fields` with the given query and
/// time range.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, patterns::DetectFields};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = DetectFields::new(loki, r#"{job="varlogs"}"#);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DetectFields {
    client: LokiClient,
    query: String,
    start: Option<String>,
    end: Option<String>,
    field_limit: Option<u64>,
    line_limit: Option<u64>,
    step: Option<String>,
}

impl DetectFields {
    /// Create a new field detection operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, patterns::DetectFields};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = DetectFields::new(loki, r#"{job="varlogs"}"#);
    /// ```
    pub fn new(client: LokiClient, query: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: None,
            end: None,
            field_limit: None,
            line_limit: None,
            step: None,
        }
    }

    /// Restrict detection to data after this timestamp.
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Restrict detection to data before this timestamp.
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }

    /// Maximum number of fields to return.
    #[must_use]
    pub fn field_limit(mut self, limit: u64) -> Self {
        self.field_limit = Some(limit);
        self
    }

    /// Maximum number of lines to inspect per stream.
    #[must_use]
    pub fn line_limit(mut self, limit: u64) -> Self {
        self.line_limit = Some(limit);
        self
    }

    /// Query resolution step.
    #[must_use]
    pub fn step(mut self, step: &str) -> Self {
        self.step = Some(step.to_owned());
        self
    }
}

#[async_trait]
impl Operation for DetectFields {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "detect_fields",
            "query": self.query,
            "start": self.start,
            "end": self.end,
            "field_limit": self.field_limit,
            "line_limit": self.line_limit,
            "step": self.step,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self
            .client
            .get("/loki/api/v1/detected_fields")
            .query(&[("query", &self.query)]);
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }
        if let Some(l) = self.field_limit {
            req = req.query(&[("field_limit", &l.to_string())]);
        }
        if let Some(l) = self.line_limit {
            req = req.query(&[("line_limit", &l.to_string())]);
        }
        if let Some(ref s) = self.step {
            req = req.query(&[("step", s)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("detect fields request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve values for a detected field.
///
/// Calls `GET /loki/api/v1/detected_field/{name}/values` with the given
/// query and time range.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, patterns::GetDetectedFieldValues};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetDetectedFieldValues::new(loki, "level", r#"{job="varlogs"}"#);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetDetectedFieldValues {
    client: LokiClient,
    field_name: String,
    query: String,
    start: Option<String>,
    end: Option<String>,
}

impl GetDetectedFieldValues {
    /// Create a new detected field values query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, patterns::GetDetectedFieldValues};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetDetectedFieldValues::new(loki, "level", r#"{job="varlogs"}"#);
    /// ```
    pub fn new(client: LokiClient, field_name: &str, query: &str) -> Self {
        Self {
            client,
            field_name: field_name.to_owned(),
            query: query.to_owned(),
            start: None,
            end: None,
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
}

#[async_trait]
impl Operation for GetDetectedFieldValues {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_detected_field_values",
            "field_name": self.field_name,
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
        validate_path_segment(&self.field_name, "field name", "loki")?;
        let mut req = self
            .client
            .get(&format!(
                "/loki/api/v1/detected_field/{}/values",
                self.field_name
            ))
            .query(&[("query", &self.query)]);
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get detected field values request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
