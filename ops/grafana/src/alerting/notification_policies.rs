//! Notification policy operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{get, put, to_value};

use super::types::NotificationPolicyOutput;

/// Get the notification policy tree.
///
/// Sends a `GET /api/v1/provisioning/policies` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::NotificationPolicyGet;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = NotificationPolicyGet::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct NotificationPolicyGet {
    url: String,
    token: String,
    http: reqwest::Client,
}

impl NotificationPolicyGet {
    /// Create a get-notification-policies operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            url: client.url("/api/v1/provisioning/policies"),
            token: client.token().to_string(),
            http: client.http().clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<NotificationPolicyOutput, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for NotificationPolicyGet {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }
}

impl TypedOperation for NotificationPolicyGet {
    type Output = NotificationPolicyOutput;
}

/// Update the notification policy tree.
///
/// Sends a `PUT /api/v1/provisioning/policies` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::NotificationPolicyUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"receiver": "email", "group_by": ["alertname"]});
/// let op = NotificationPolicyUpdate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct NotificationPolicyUpdate {
    url: String,
    token: String,
    http: reqwest::Client,
    body: Value,
}

impl NotificationPolicyUpdate {
    /// Create an update-notification-policies operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            url: client.url("/api/v1/provisioning/policies"),
            token: client.token().to_string(),
            http: client.http().clone(),
            body,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        put::<_, Value>(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for NotificationPolicyUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(self.body.clone())
    }
}
