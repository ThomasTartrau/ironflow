//! Metrics ingestion operations.
//!
//! These operations push metric samples into Mimir via the remote write,
//! OTLP, and InfluxDB-compatible endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Push metrics via Prometheus remote write protocol.
///
/// Calls `POST /api/v1/push` with the given payload. The payload should
/// be a Snappy-compressed protobuf or JSON body following the Prometheus
/// remote write specification.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, ingest::RemoteWrite};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = RemoteWrite::new(mimir, vec![1, 2, 3]);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RemoteWrite {
    client: MimirClient,
    payload: Vec<u8>,
}

impl RemoteWrite {
    /// Create a new remote write operation with a raw payload.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, ingest::RemoteWrite};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = RemoteWrite::new(mimir, vec![1, 2, 3]);
    /// ```
    pub fn new(client: MimirClient, payload: Vec<u8>) -> Self {
        Self { client, payload }
    }
}

#[async_trait]
impl Operation for RemoteWrite {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "remote_write",
            "payload_size": self.payload.len(),
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/api/v1/push")
            .header("Content-Type", "application/x-protobuf")
            .header("Content-Encoding", "snappy")
            .header("X-Prometheus-Remote-Write-Version", "0.1.0")
            .body(self.payload.clone())
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("remote write request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Push metrics via the OTLP (OpenTelemetry) endpoint.
///
/// Calls `POST /otlp/v1/metrics` with the given JSON payload following
/// the OpenTelemetry Metrics Data Model.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, ingest::OtlpMetricsWrite};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = OtlpMetricsWrite::new(mimir, json!({"resourceMetrics": []}));
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct OtlpMetricsWrite {
    client: MimirClient,
    payload: Value,
}

impl OtlpMetricsWrite {
    /// Create a new OTLP metrics write operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, ingest::OtlpMetricsWrite};
    /// use serde_json::json;
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = OtlpMetricsWrite::new(mimir, json!({"resourceMetrics": []}));
    /// ```
    pub fn new(client: MimirClient, payload: Value) -> Self {
        Self { client, payload }
    }
}

#[async_trait]
impl Operation for OtlpMetricsWrite {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "otlp_metrics_write"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/otlp/v1/metrics")
            .json(&self.payload)
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("otlp metrics write request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Push metrics via the InfluxDB-compatible write endpoint.
///
/// Calls `POST /api/v1/push/influx/write` with InfluxDB line protocol data.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, ingest::InfluxWrite};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = InfluxWrite::new(mimir, "cpu,host=A value=0.5");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct InfluxWrite {
    client: MimirClient,
    line_protocol: String,
}

impl InfluxWrite {
    /// Create a new InfluxDB-compatible write operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, ingest::InfluxWrite};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = InfluxWrite::new(mimir, "cpu,host=A value=0.5");
    /// ```
    pub fn new(client: MimirClient, line_protocol: &str) -> Self {
        Self {
            client,
            line_protocol: line_protocol.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for InfluxWrite {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "influx_write"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/api/v1/push/influx/write")
            .header("Content-Type", "text/plain")
            .body(self.line_protocol.clone())
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("influx write request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
