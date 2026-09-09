//! Admin operations.
//!
//! Provides server statistics, alert pausing, and health checks
//! via the `/api/admin/` and `/api/health` endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;

/// Server-wide statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminStatsOutput {
    /// Total users.
    pub users: Option<u64>,
    /// Total organizations.
    pub orgs: Option<u64>,
    /// Total dashboards.
    pub dashboards: Option<u64>,
    /// Total snapshots.
    pub snapshots: Option<u64>,
    /// Total tags.
    pub tags: Option<u64>,
    /// Total data sources.
    pub datasources: Option<u64>,
    /// Total playlists.
    pub playlists: Option<u64>,
    /// Total stars.
    pub stars: Option<u64>,
    /// Total alerts.
    pub alerts: Option<u64>,
    /// Active users.
    pub active_users: Option<u64>,
}

/// Health check response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthOutput {
    /// Git commit hash.
    pub commit: Option<String>,
    /// Database status.
    pub database: Option<String>,
    /// Grafana version.
    pub version: Option<String>,
}

/// Get server statistics.
///
/// Sends a `GET /api/admin/stats` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::admin::AdminGetStats;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AdminGetStats::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AdminGetStats {
    client: GrafanaClient,
}

impl AdminGetStats {
    /// Create a get-stats operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            client: client.clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<AdminStatsOutput, OperationError> {
        self.client.get_json("/api/admin/stats").await
    }
}

#[async_trait]
impl Operation for AdminGetStats {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "endpoint": "admin/stats" }))
    }
}

impl TypedOperation for AdminGetStats {
    type Output = AdminStatsOutput;
}

/// Set the pause state of all alerts.
///
/// Sends a `POST /api/admin/pause-all-alerts` request with the given
/// `paused` flag (`true` to pause, `false` to unpause).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::admin::AdminSetAlertsPause;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AdminSetAlertsPause::new(&grafana, true);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AdminSetAlertsPause {
    client: GrafanaClient,
    paused: bool,
}

/// Pause all alerts (convenience alias for [`AdminSetAlertsPause`]).
pub type AdminPauseAllAlerts = AdminSetAlertsPause;

/// Unpause all alerts (convenience alias for [`AdminSetAlertsPause`]).
pub type AdminUnpauseAllAlerts = AdminSetAlertsPause;

impl AdminSetAlertsPause {
    /// Create a set-alerts-pause operation.
    pub fn new(client: &GrafanaClient, paused: bool) -> Self {
        Self {
            client: client.clone(),
            paused,
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
                "/api/admin/pause-all-alerts",
                &serde_json::json!({ "paused": self.paused }),
            )
            .await
    }
}

#[async_trait]
impl Operation for AdminSetAlertsPause {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "paused": self.paused }))
    }
}

/// Get Grafana server health.
///
/// Sends a `GET /api/health` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::admin::AdminGetHealth;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AdminGetHealth::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AdminGetHealth {
    client: GrafanaClient,
}

impl AdminGetHealth {
    /// Create a get-health operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            client: client.clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<HealthOutput, OperationError> {
        self.client.get_json("/api/health").await
    }
}

#[async_trait]
impl Operation for AdminGetHealth {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "endpoint": "health" }))
    }
}

impl TypedOperation for AdminGetHealth {
    type Output = HealthOutput;
}
