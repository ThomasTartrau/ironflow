//! Log deletion operations.
//!
//! These operations manage Loki's log deletion API, which allows
//! requesting deletion of log entries matching a query over a time range.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::check_response;

/// Request deletion of log entries matching a query.
///
/// Calls `POST /loki/api/v1/delete` with query, start, and end parameters.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, delete::CreateDeleteRequest};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = CreateDeleteRequest::new(loki, r#"{job="test"}"#, "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CreateDeleteRequest {
    client: LokiClient,
    query: String,
    start: String,
    end: String,
}

impl CreateDeleteRequest {
    /// Create a new log deletion request.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, delete::CreateDeleteRequest};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = CreateDeleteRequest::new(loki, r#"{job="test"}"#, "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");
    /// ```
    pub fn new(client: LokiClient, query: &str, start: &str, end: &str) -> Self {
        Self {
            client,
            query: query.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for CreateDeleteRequest {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "create_delete_request",
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
        let response = self
            .client
            .post("/loki/api/v1/delete")
            .query(&[
                ("query", &self.query),
                ("start", &self.start),
                ("end", &self.end),
            ])
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("create delete request failed: {e}"),
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

/// List all pending and processed delete requests.
///
/// Calls `GET /loki/api/v1/delete`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, delete::ListDeleteRequests};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = ListDeleteRequests::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ListDeleteRequests {
    client: LokiClient,
}

impl ListDeleteRequests {
    /// Create a new operation to list delete requests.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, delete::ListDeleteRequests};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = ListDeleteRequests::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for ListDeleteRequests {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "list_delete_requests"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/loki/api/v1/delete")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("list delete requests failed: {e}"),
            })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Cancel a pending delete request.
///
/// Calls `DELETE /loki/api/v1/delete` with the `request_id` parameter.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, delete::CancelDeleteRequest};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = CancelDeleteRequest::new(loki, "req-123");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CancelDeleteRequest {
    client: LokiClient,
    request_id: String,
}

impl CancelDeleteRequest {
    /// Create a new cancel operation for a delete request.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, delete::CancelDeleteRequest};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = CancelDeleteRequest::new(loki, "req-123");
    /// ```
    pub fn new(client: LokiClient, request_id: &str) -> Self {
        Self {
            client,
            request_id: request_id.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for CancelDeleteRequest {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "cancel_delete_request",
            "request_id": self.request_id,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .delete("/loki/api/v1/delete")
            .query(&[("request_id", &self.request_id)])
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("cancel delete request failed: {e}"),
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
