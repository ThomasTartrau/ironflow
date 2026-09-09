//! Alert rule operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{delete, get, post, put, to_value};

use super::types::AlertRuleOutput;

/// Get an alert rule by UID.
///
/// Sends a `GET /api/v1/provisioning/alert-rules/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::AlertRuleGet;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AlertRuleGet::new(&grafana, "rule-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AlertRuleGet {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl AlertRuleGet {
    /// Create a get-alert-rule operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/v1/provisioning/alert-rules/{uid}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<AlertRuleOutput, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for AlertRuleGet {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid }))
    }
}

impl TypedOperation for AlertRuleGet {
    type Output = AlertRuleOutput;
}

/// List all alert rules.
///
/// Sends a `GET /api/v1/provisioning/alert-rules` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::AlertRuleList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AlertRuleList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AlertRuleList {
    url: String,
    token: String,
    http: reqwest::Client,
}

impl AlertRuleList {
    /// Create a list-alert-rules operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            url: client.url("/api/v1/provisioning/alert-rules"),
            token: client.token().to_string(),
            http: client.http().clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<AlertRuleOutput>, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for AlertRuleList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }
}

impl TypedOperation for AlertRuleList {
    type Output = Vec<AlertRuleOutput>;
}

/// Create an alert rule.
///
/// Sends a `POST /api/v1/provisioning/alert-rules` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::AlertRuleCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"title": "CPU high", "condition": "A", "folderUID": "abc"});
/// let op = AlertRuleCreate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AlertRuleCreate {
    url: String,
    token: String,
    http: reqwest::Client,
    body: Value,
}

impl AlertRuleCreate {
    /// Create a create-alert-rule operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            url: client.url("/api/v1/provisioning/alert-rules"),
            token: client.token().to_string(),
            http: client.http().clone(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<AlertRuleOutput, OperationError> {
        post(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for AlertRuleCreate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(self.body.clone())
    }
}

impl TypedOperation for AlertRuleCreate {
    type Output = AlertRuleOutput;
}

/// Update an alert rule.
///
/// Sends a `PUT /api/v1/provisioning/alert-rules/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::AlertRuleUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"title": "CPU high v2"});
/// let op = AlertRuleUpdate::new(&grafana, "rule-uid", body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AlertRuleUpdate {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
    body: Value,
}

impl AlertRuleUpdate {
    /// Create an update-alert-rule operation.
    pub fn new(client: &GrafanaClient, uid: &str, body: Value) -> Self {
        Self {
            url: client.url(&format!("/api/v1/provisioning/alert-rules/{uid}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<AlertRuleOutput, OperationError> {
        put(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for AlertRuleUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid, "body": self.body }))
    }
}

impl TypedOperation for AlertRuleUpdate {
    type Output = AlertRuleOutput;
}

/// Delete an alert rule.
///
/// Sends a `DELETE /api/v1/provisioning/alert-rules/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::AlertRuleDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AlertRuleDelete::new(&grafana, "rule-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AlertRuleDelete {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl AlertRuleDelete {
    /// Create a delete-alert-rule operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/v1/provisioning/alert-rules/{uid}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        delete(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for AlertRuleDelete {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid }))
    }
}
