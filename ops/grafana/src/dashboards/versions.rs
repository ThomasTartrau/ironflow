//! Dashboard versioning operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::DashboardVersion;

/// Get all versions of a dashboard.
///
/// Sends a `GET /api/dashboards/id/{id}/versions` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardGetVersions;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DashboardGetVersions::new(&grafana, 42);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardGetVersions {
    client: GrafanaClient,
    dashboard_id: u64,
}

impl DashboardGetVersions {
    /// Create a get-versions operation.
    pub fn new(client: &GrafanaClient, dashboard_id: u64) -> Self {
        Self {
            client: client.clone(),
            dashboard_id,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<DashboardVersion>, OperationError> {
        self.client
            .get_json(&format!(
                "/api/dashboards/id/{}/versions",
                self.dashboard_id
            ))
            .await
    }
}

#[async_trait]
impl Operation for DashboardGetVersions {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "dashboard_id": self.dashboard_id }))
    }
}

impl TypedOperation for DashboardGetVersions {
    type Output = Vec<DashboardVersion>;
}

/// Get a specific version of a dashboard.
///
/// Sends a `GET /api/dashboards/id/{id}/versions/{version}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardGetVersion;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DashboardGetVersion::new(&grafana, 42, 3);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardGetVersion {
    client: GrafanaClient,
    dashboard_id: u64,
    version: u64,
}

impl DashboardGetVersion {
    /// Create a get-version operation.
    pub fn new(client: &GrafanaClient, dashboard_id: u64, version: u64) -> Self {
        Self {
            client: client.clone(),
            dashboard_id,
            version,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .get_json(&format!(
                "/api/dashboards/id/{}/versions/{}",
                self.dashboard_id, self.version
            ))
            .await
    }
}

#[async_trait]
impl Operation for DashboardGetVersion {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "dashboard_id": self.dashboard_id,
            "version": self.version,
        }))
    }
}

/// Restore a dashboard to a previous version.
///
/// Sends a `POST /api/dashboards/id/{id}/restore` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::dashboards::DashboardRestoreVersion;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DashboardRestoreVersion::new(&grafana, 42, 2);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DashboardRestoreVersion {
    client: GrafanaClient,
    dashboard_id: u64,
    version: u64,
}

impl DashboardRestoreVersion {
    /// Create a restore-version operation.
    pub fn new(client: &GrafanaClient, dashboard_id: u64, version: u64) -> Self {
        Self {
            client: client.clone(),
            dashboard_id,
            version,
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
                &format!("/api/dashboards/id/{}/restore", self.dashboard_id),
                &serde_json::json!({ "version": self.version }),
            )
            .await
    }
}

#[async_trait]
impl Operation for DashboardRestoreVersion {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "dashboard_id": self.dashboard_id,
            "version": self.version,
        }))
    }
}
