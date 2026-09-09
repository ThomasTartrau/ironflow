//! Organization admin operations (server admin level).

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::OrgOutput;

/// List all organizations (admin).
///
/// Sends a `GET /api/orgs` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgList {
    client: GrafanaClient,
}

impl OrgList {
    /// Create a list-orgs operation.
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
    pub async fn run(&self) -> Result<Vec<OrgOutput>, OperationError> {
        self.client.get_json("/api/orgs").await
    }
}

#[async_trait]
impl Operation for OrgList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }
}

impl TypedOperation for OrgList {
    type Output = Vec<OrgOutput>;
}

/// Get an organization by ID (admin).
///
/// Sends a `GET /api/orgs/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgGet;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgGet::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgGet {
    client: GrafanaClient,
    id: u64,
}

impl OrgGet {
    /// Create a get-org operation.
    pub fn new(client: &GrafanaClient, id: u64) -> Self {
        Self {
            client: client.clone(),
            id,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<OrgOutput, OperationError> {
        self.client
            .get_json(&format!("/api/orgs/{}", self.id))
            .await
    }
}

#[async_trait]
impl Operation for OrgGet {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "id": self.id }))
    }
}

impl TypedOperation for OrgGet {
    type Output = OrgOutput;
}

/// Create an organization (admin).
///
/// Sends a `POST /api/orgs` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgCreate::new(&grafana, "New Org");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgCreate {
    client: GrafanaClient,
    name: String,
}

impl OrgCreate {
    /// Create a new-org operation.
    pub fn new(client: &GrafanaClient, name: &str) -> Self {
        Self {
            client: client.clone(),
            name: name.to_string(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .post_json("/api/orgs", &serde_json::json!({ "name": self.name }))
            .await
    }
}

#[async_trait]
impl Operation for OrgCreate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "name": self.name }))
    }
}

/// Delete an organization (admin).
///
/// Sends a `DELETE /api/orgs/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgDelete::new(&grafana, 2);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgDelete {
    client: GrafanaClient,
    id: u64,
}

impl OrgDelete {
    /// Create a delete-org operation.
    pub fn new(client: &GrafanaClient, id: u64) -> Self {
        Self {
            client: client.clone(),
            id,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .delete_json(&format!("/api/orgs/{}", self.id))
            .await
    }
}

#[async_trait]
impl Operation for OrgDelete {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "id": self.id }))
    }
}
