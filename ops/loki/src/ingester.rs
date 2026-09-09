//! Ingester lifecycle operations.
//!
//! These operations control the Loki ingester's lifecycle: flushing data
//! to long-term storage, and managing graceful shutdown sequences.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;
use serde_json::json;

use crate::LokiClient;
use crate::error::check_response;

/// Flush all in-memory chunks to long-term storage.
///
/// Calls `POST /flush`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, ingester::Flush};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = Flush::new(loki);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Flush {
    client: LokiClient,
}

impl Flush {
    /// Create a new flush operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, ingester::Flush};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = Flush::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for Flush {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "flush"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .post("/flush")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("flush request failed: {e}"),
                })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"status": "success", "body": text.as_ref()}))
    }
}

/// Prepare the ingester for shutdown.
///
/// Calls `POST /ingester/prepare_shutdown`. The ingester will stop accepting
/// new writes and flush existing data.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, ingester::PrepareShutdown};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = PrepareShutdown::new(loki);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct PrepareShutdown {
    client: LokiClient,
}

impl PrepareShutdown {
    /// Create a new prepare shutdown operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, ingester::PrepareShutdown};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = PrepareShutdown::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for PrepareShutdown {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "prepare_shutdown"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/ingester/prepare_shutdown")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("prepare shutdown request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"status": "success", "body": text.as_ref()}))
    }
}

/// Cancel a pending shutdown.
///
/// Calls `DELETE /ingester/prepare_shutdown`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, ingester::CancelShutdown};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = CancelShutdown::new(loki);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CancelShutdown {
    client: LokiClient,
}

impl CancelShutdown {
    /// Create a new cancel shutdown operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, ingester::CancelShutdown};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = CancelShutdown::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for CancelShutdown {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "cancel_shutdown"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .delete("/ingester/prepare_shutdown")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("cancel shutdown request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"status": "success", "body": text.as_ref()}))
    }
}

/// Shut down the ingester immediately.
///
/// Calls `POST /ingester/shutdown`. This is a destructive operation that
/// immediately terminates the ingester.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, ingester::Shutdown};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = Shutdown::new(loki);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Shutdown {
    client: LokiClient,
}

impl Shutdown {
    /// Create a new immediate shutdown operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, ingester::Shutdown};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = Shutdown::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for Shutdown {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "shutdown"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/ingester/shutdown")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("shutdown request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"status": "success", "body": text.as_ref()}))
    }
}
