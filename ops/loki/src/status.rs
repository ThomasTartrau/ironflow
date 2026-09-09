//! Loki instance status and configuration operations.
//!
//! These operations query and modify the running Loki instance's
//! status, log level, metrics, and configuration.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::check_response;

/// Check if Loki is ready to accept requests.
///
/// Calls `GET /ready`. Returns `200 OK` when Loki is ready.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, status::GetReady};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetReady::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetReady {
    client: LokiClient,
}

impl GetReady {
    /// Create a new readiness check operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, status::GetReady};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetReady::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetReady {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_ready"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or Loki is not ready.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/ready")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("readiness check failed: {e}"),
                })?;

        let status = response.status().as_u16();
        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ready": true, "status": status, "body": text.as_ref()}))
    }
}

/// Get the current log level.
///
/// Calls `GET /log_level`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, status::GetLogLevel};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetLogLevel::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLogLevel {
    client: LokiClient,
}

impl GetLogLevel {
    /// Create a new log level query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, status::GetLogLevel};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetLogLevel::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetLogLevel {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_log_level"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/log_level")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("get log level request failed: {e}"),
                })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Set the log level at runtime.
///
/// Calls `POST /log_level` with the desired level.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, status::SetLogLevel};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = SetLogLevel::new(loki, "debug");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct SetLogLevel {
    client: LokiClient,
    level: String,
}

impl SetLogLevel {
    /// Create a new log level update operation.
    ///
    /// Valid levels: `"debug"`, `"info"`, `"warn"`, `"error"`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, status::SetLogLevel};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = SetLogLevel::new(loki, "debug");
    /// ```
    pub fn new(client: LokiClient, level: &str) -> Self {
        Self {
            client,
            level: level.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for SetLogLevel {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "set_log_level",
            "level": self.level,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/log_level")
            .json(&json!({"log_level": self.level}))
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("set log level request failed: {e}"),
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

/// Retrieve Prometheus metrics from Loki.
///
/// Calls `GET /metrics`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, status::GetMetrics};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetMetrics::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetMetrics {
    client: LokiClient,
}

impl GetMetrics {
    /// Create a new metrics query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, status::GetMetrics};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetMetrics::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetMetrics {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_metrics"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/metrics")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("get metrics request failed: {e}"),
                })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"metrics": text.as_ref()}))
    }
}

/// Retrieve Loki's runtime configuration.
///
/// Calls `GET /config`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, status::GetConfig};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetConfig::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetConfig {
    client: LokiClient,
}

impl GetConfig {
    /// Create a new config query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, status::GetConfig};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetConfig::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetConfig {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_config"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/config")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("get config request failed: {e}"),
                })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"config": text.as_ref()}))
    }
}
