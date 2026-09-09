//! Alert rule group operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::AlertRuleGroupOutput;

/// Get an alert rule group.
///
/// Sends a `GET /api/v1/provisioning/folder/{folder_uid}/rule-groups/{group}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::AlertRuleGroupGet;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AlertRuleGroupGet::new(&grafana, "folder-uid", "my-group");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AlertRuleGroupGet {
    client: GrafanaClient,
    folder_uid: String,
    group: String,
}

impl AlertRuleGroupGet {
    /// Create a get-alert-rule-group operation.
    pub fn new(client: &GrafanaClient, folder_uid: &str, group: &str) -> Self {
        Self {
            client: client.clone(),
            folder_uid: folder_uid.to_string(),
            group: group.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<AlertRuleGroupOutput, OperationError> {
        self.client
            .get_json(&format!(
                "/api/v1/provisioning/folder/{}/rule-groups/{}",
                self.folder_uid, self.group
            ))
            .await
    }
}

#[async_trait]
impl Operation for AlertRuleGroupGet {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "folder_uid": self.folder_uid,
            "group": self.group,
        }))
    }
}

impl TypedOperation for AlertRuleGroupGet {
    type Output = AlertRuleGroupOutput;
}

/// Update an alert rule group.
///
/// Sends a `PUT /api/v1/provisioning/folder/{folder_uid}/rule-groups/{group}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::AlertRuleGroupUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"interval": "5m", "rules": []});
/// let op = AlertRuleGroupUpdate::new(&grafana, "folder-uid", "my-group", body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AlertRuleGroupUpdate {
    client: GrafanaClient,
    folder_uid: String,
    group: String,
    body: Value,
}

impl AlertRuleGroupUpdate {
    /// Create an update-alert-rule-group operation.
    pub fn new(client: &GrafanaClient, folder_uid: &str, group: &str, body: Value) -> Self {
        Self {
            client: client.clone(),
            folder_uid: folder_uid.to_string(),
            group: group.to_string(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<AlertRuleGroupOutput, OperationError> {
        self.client
            .put_json(
                &format!(
                    "/api/v1/provisioning/folder/{}/rule-groups/{}",
                    self.folder_uid, self.group
                ),
                &self.body,
            )
            .await
    }
}

#[async_trait]
impl Operation for AlertRuleGroupUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "folder_uid": self.folder_uid,
            "group": self.group,
            "body": self.body,
        }))
    }
}

impl TypedOperation for AlertRuleGroupUpdate {
    type Output = AlertRuleGroupOutput;
}
