//! Dashboard CRUD operations (Create, Get, Update, Delete).

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::{DashboardGetOutput, DashboardSaveOutput};

/// Save (create or update) a dashboard.
///
/// Sends a `POST /api/dashboards/db` request. Grafana uses the same
/// endpoint for both creation and updates; set `"overwrite": true` or
/// include an existing `"id"` in the body to update.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardSave;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"dashboard": {"title": "New"}, "overwrite": false});
/// let op = DashboardSave::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardSave {
    client: GrafanaClient,
    body: Value,
}

/// Create a new dashboard (convenience alias for [`DashboardSave`]).
pub type DashboardCreate = DashboardSave;

/// Update an existing dashboard (convenience alias for [`DashboardSave`]).
pub type DashboardUpdate = DashboardSave;

impl DashboardSave {
    /// Create a dashboard save operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            client: client.clone(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<DashboardSaveOutput, OperationError> {
        self.client
            .post_json("/api/dashboards/db", &self.body)
            .await
    }
}

#[async_trait]
impl Operation for DashboardSave {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(self.body.clone())
    }
}

impl TypedOperation for DashboardSave {
    type Output = DashboardSaveOutput;
}

/// Get a dashboard by UID.
///
/// Sends a `GET /api/dashboards/uid/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardGet;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DashboardGet::new(&grafana, "my-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardGet {
    client: GrafanaClient,
    uid: String,
}

impl DashboardGet {
    /// Create a get-dashboard operation.
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
    pub async fn run(&self) -> Result<DashboardGetOutput, OperationError> {
        self.client
            .get_json(&format!("/api/dashboards/uid/{}", self.uid))
            .await
    }
}

#[async_trait]
impl Operation for DashboardGet {
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

impl TypedOperation for DashboardGet {
    type Output = DashboardGetOutput;
}

/// Delete a dashboard by UID.
///
/// Sends a `DELETE /api/dashboards/uid/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DashboardDelete::new(&grafana, "my-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardDelete {
    client: GrafanaClient,
    uid: String,
}

impl DashboardDelete {
    /// Create a delete-dashboard operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            client: client.clone(),
            uid: uid.to_string(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .delete_json(&format!("/api/dashboards/uid/{}", self.uid))
            .await
    }
}

#[async_trait]
impl Operation for DashboardDelete {
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
