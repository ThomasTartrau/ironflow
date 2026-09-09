//! RBAC (Role-Based Access Control) operations.
//!
//! Provides role management for Grafana via the `/api/access-control/` endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{delete, get, post, put, to_value};

/// Role metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleOutput {
    /// Role UID.
    pub uid: Option<String>,
    /// Role name.
    pub name: Option<String>,
    /// Role description.
    pub description: Option<String>,
    /// Role version.
    pub version: Option<u64>,
    /// Whether the role is global.
    pub global: Option<bool>,
}

/// A role assignment entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleAssignment {
    /// Role UID.
    pub role_uid: Option<String>,
    /// Assignment scope.
    pub scope: Option<String>,
}

/// Get all roles.
///
/// Sends a `GET /api/access-control/roles` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::rbac::RbacGetRoles;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = RbacGetRoles::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct RbacGetRoles {
    url: String,
    token: String,
    http: reqwest::Client,
}

impl RbacGetRoles {
    /// Create a get-roles operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            url: client.url("/api/access-control/roles"),
            token: client.token().to_string(),
            http: client.http().clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<RoleOutput>, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for RbacGetRoles {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "endpoint": "access-control/roles" }))
    }
}

impl TypedOperation for RbacGetRoles {
    type Output = Vec<RoleOutput>;
}

/// Get a role by UID.
///
/// Sends a `GET /api/access-control/roles/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::rbac::RbacGetRole;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = RbacGetRole::new(&grafana, "role-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct RbacGetRole {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl RbacGetRole {
    /// Create a get-role operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/access-control/roles/{uid}")),
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
    pub async fn run(&self) -> Result<RoleOutput, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for RbacGetRole {
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

impl TypedOperation for RbacGetRole {
    type Output = RoleOutput;
}

/// Create a role.
///
/// Sends a `POST /api/access-control/roles` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::rbac::RbacCreateRole;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "custom:reader", "permissions": []});
/// let op = RbacCreateRole::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct RbacCreateRole {
    url: String,
    token: String,
    http: reqwest::Client,
    body: Value,
}

impl RbacCreateRole {
    /// Create a role creation operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            url: client.url("/api/access-control/roles"),
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
    pub async fn run(&self) -> Result<RoleOutput, OperationError> {
        post(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for RbacCreateRole {
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

impl TypedOperation for RbacCreateRole {
    type Output = RoleOutput;
}

/// Update a role.
///
/// Sends a `PUT /api/access-control/roles/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::rbac::RbacUpdateRole;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "custom:writer", "permissions": []});
/// let op = RbacUpdateRole::new(&grafana, "role-uid", body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct RbacUpdateRole {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
    body: Value,
}

impl RbacUpdateRole {
    /// Create an update-role operation.
    pub fn new(client: &GrafanaClient, uid: &str, body: Value) -> Self {
        Self {
            url: client.url(&format!("/api/access-control/roles/{uid}")),
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
    pub async fn run(&self) -> Result<RoleOutput, OperationError> {
        put(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for RbacUpdateRole {
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

impl TypedOperation for RbacUpdateRole {
    type Output = RoleOutput;
}

/// Delete a role.
///
/// Sends a `DELETE /api/access-control/roles/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::rbac::RbacDeleteRole;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = RbacDeleteRole::new(&grafana, "role-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct RbacDeleteRole {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl RbacDeleteRole {
    /// Create a delete-role operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/access-control/roles/{uid}")),
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
impl Operation for RbacDeleteRole {
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

/// Get role assignments.
///
/// Sends a `GET /api/access-control/roles/{uid}/assignments` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::rbac::RbacGetRoleAssignments;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = RbacGetRoleAssignments::new(&grafana, "role-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct RbacGetRoleAssignments {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl RbacGetRoleAssignments {
    /// Create a get-assignments operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/access-control/roles/{uid}/assignments")),
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
    pub async fn run(&self) -> Result<Vec<RoleAssignment>, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for RbacGetRoleAssignments {
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

impl TypedOperation for RbacGetRoleAssignments {
    type Output = Vec<RoleAssignment>;
}
