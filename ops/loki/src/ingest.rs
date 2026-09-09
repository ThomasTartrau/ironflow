//! Log ingestion operations.
//!
//! These operations push log entries into Loki via the push API endpoints.

use std::collections::HashMap;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde::Serialize;
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::check_response;

/// Push log entries to Loki.
///
/// Calls `POST /loki/api/v1/push` with a JSON payload containing streams
/// of log entries.
///
/// # Payload format
///
/// The payload must follow the Loki push API format:
///
/// ```json
/// {
///   "streams": [
///     {
///       "stream": { "label": "value" },
///       "values": [ ["<unix_epoch_ns>", "<log_line>"] ]
///     }
///   ]
/// }
/// ```
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, ingest::PushLogs};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let payload = json!({
///     "streams": [{
///         "stream": { "job": "test" },
///         "values": [["1234567890000000000", "hello world"]]
///     }]
/// });
/// let op = PushLogs::new(loki, payload);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct PushLogs {
    client: LokiClient,
    payload: Value,
}

impl PushLogs {
    /// Create a new push operation with the given JSON payload.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, ingest::PushLogs};
    /// use serde_json::json;
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let payload = json!({
    ///     "streams": [{
    ///         "stream": { "job": "test" },
    ///         "values": [["1234567890000000000", "hello"]]
    ///     }]
    /// });
    /// let op = PushLogs::new(loki, payload);
    /// ```
    pub fn new(client: LokiClient, payload: Value) -> Self {
        Self { client, payload }
    }
}

#[async_trait]
impl Operation for PushLogs {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "push_logs",
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/loki/api/v1/push")
            .json(&self.payload)
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("push logs request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Push log entries to Loki via the OTLP (OpenTelemetry) endpoint.
///
/// Calls `POST /otlp/v1/logs` with the given payload. The payload should
/// follow the OpenTelemetry Logs Data Model.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, ingest::PushLogsOtlp};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let payload = json!({"resourceLogs": []});
/// let op = PushLogsOtlp::new(loki, payload);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct PushLogsOtlp {
    client: LokiClient,
    payload: Value,
}

impl PushLogsOtlp {
    /// Create a new OTLP push operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, ingest::PushLogsOtlp};
    /// use serde_json::json;
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = PushLogsOtlp::new(loki, json!({"resourceLogs": []}));
    /// ```
    pub fn new(client: LokiClient, payload: Value) -> Self {
        Self { client, payload }
    }
}

#[async_trait]
impl Operation for PushLogsOtlp {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "push_logs_otlp",
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/otlp/v1/logs")
            .json(&self.payload)
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("push logs otlp request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// A single stream of log entries for the push payload.
///
/// Helper type for constructing push payloads. Not used directly as an
/// operation.
///
/// # Examples
///
/// ```
/// use ironflow_ops_loki::ingest::Stream;
/// use std::collections::HashMap;
///
/// let stream = Stream {
///     stream: HashMap::from([("job".into(), "test".into())]),
///     values: vec![("1234567890000000000".into(), "hello".into())],
/// };
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct Stream {
    /// Label key-value pairs identifying this stream.
    pub stream: HashMap<String, String>,
    /// Log entries as `(timestamp_ns, line)` pairs.
    pub values: Vec<(String, String)>,
}
