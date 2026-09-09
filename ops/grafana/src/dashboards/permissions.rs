//! Dashboard permission operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::DashboardPermission;

/// Get permissions for a dashboard.
///
/// Sends a `GET /api/dashboards/uid/{uid}/permissions` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardGetPermissions;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DashboardGetPermissions::new(&grafana, "my-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardGetPermissions {
    client: GrafanaClient,
    uid: String,
}

impl DashboardGetPermissions {
    /// Create a get-permissions operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            client: client.clone(),
            uid: uid.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<DashboardPermission>, OperationError> {
        self.client
            .get_json(&format!("/api/dashboards/uid/{}/permissions", self.uid))
            .await
    }
}

#[async_trait]
impl Operation for DashboardGetPermissions {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid }))
    }
}

impl TypedOperation for DashboardGetPermissions {
    type Output = Vec<DashboardPermission>;
}

/// Update permissions for a dashboard.
///
/// Sends a `POST /api/dashboards/uid/{uid}/permissions` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardUpdatePermissions;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let items = json!({"items": [{"role": "Viewer", "permission": 1}]});
/// let op = DashboardUpdatePermissions::new(&grafana, "my-uid", items);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardUpdatePermissions {
    client: GrafanaClient,
    uid: String,
    body: Value,
}

impl DashboardUpdatePermissions {
    /// Create an update-permissions operation.
    pub fn new(client: &GrafanaClient, uid: &str, body: Value) -> Self {
        Self {
            client: client.clone(),
            uid: uid.to_string(),
            body,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .post_json(
                &format!("/api/dashboards/uid/{}/permissions", self.uid),
                &self.body,
            )
            .await
    }
}

#[async_trait]
impl Operation for DashboardUpdatePermissions {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid, "body": self.body }))
    }
}
