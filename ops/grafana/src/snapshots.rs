//! Snapshot operations.
//!
//! Provides create, list, get, and delete for Grafana dashboard snapshots
//! via the `/api/snapshots/` and `/api/dashboard/snapshots` endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{delete, get, post, to_value};

/// Response from snapshot creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotCreateOutput {
    /// Snapshot key.
    pub key: Option<String>,
    /// Key used to delete the snapshot.
    pub delete_key: Option<String>,
    /// Snapshot URL.
    pub url: Option<String>,
    /// URL to delete the snapshot.
    pub delete_url: Option<String>,
}

/// A snapshot list entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotListItem {
    /// Snapshot key.
    pub key: Option<String>,
    /// Snapshot name.
    pub name: Option<String>,
    /// Whether the snapshot is external.
    pub external: Option<bool>,
    /// Expiration date.
    pub expires: Option<String>,
}

/// Create a dashboard snapshot.
///
/// Sends a `POST /api/snapshots` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::snapshots::SnapshotCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"dashboard": {"title": "Snap"}, "expires": 3600});
/// let op = SnapshotCreate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct SnapshotCreate {
    url: String,
    token: String,
    http: reqwest::Client,
    body: Value,
}

impl SnapshotCreate {
    /// Create a snapshot creation operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            url: client.url("/api/snapshots"),
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
    pub async fn run(&self) -> Result<SnapshotCreateOutput, OperationError> {
        post(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for SnapshotCreate {
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

impl TypedOperation for SnapshotCreate {
    type Output = SnapshotCreateOutput;
}

/// List dashboard snapshots.
///
/// Sends a `GET /api/dashboard/snapshots` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::snapshots::SnapshotList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = SnapshotList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct SnapshotList {
    url: String,
    token: String,
    http: reqwest::Client,
}

impl SnapshotList {
    /// Create a list operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            url: client.url("/api/dashboard/snapshots"),
            token: client.token().to_string(),
            http: client.http().clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<SnapshotListItem>, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for SnapshotList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "endpoint": "dashboard/snapshots" }))
    }
}

impl TypedOperation for SnapshotList {
    type Output = Vec<SnapshotListItem>;
}

/// Get a snapshot by key.
///
/// Sends a `GET /api/snapshots/{key}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::snapshots::SnapshotGetByKey;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = SnapshotGetByKey::new(&grafana, "abc123");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct SnapshotGetByKey {
    url: String,
    token: String,
    http: reqwest::Client,
    key: String,
}

impl SnapshotGetByKey {
    /// Create a get-by-key operation.
    pub fn new(client: &GrafanaClient, key: &str) -> Self {
        Self {
            url: client.url(&format!("/api/snapshots/{key}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            key: key.to_string(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        get::<Value>(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for SnapshotGetByKey {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "key": self.key }))
    }
}

/// Delete a snapshot by key.
///
/// Sends a `DELETE /api/snapshots/{key}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::snapshots::SnapshotDeleteByKey;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = SnapshotDeleteByKey::new(&grafana, "abc123");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct SnapshotDeleteByKey {
    url: String,
    token: String,
    http: reqwest::Client,
    key: String,
}

impl SnapshotDeleteByKey {
    /// Create a delete-by-key operation.
    pub fn new(client: &GrafanaClient, key: &str) -> Self {
        Self {
            url: client.url(&format!("/api/snapshots/{key}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            key: key.to_string(),
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
impl Operation for SnapshotDeleteByKey {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "key": self.key }))
    }
}
