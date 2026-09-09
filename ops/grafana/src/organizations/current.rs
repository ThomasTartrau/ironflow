//! Current organization operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::{OrgOutput, OrgUserOutput};

/// Get the current organization.
///
/// Sends a `GET /api/org` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgGetCurrent;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgGetCurrent::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgGetCurrent {
    client: GrafanaClient,
}

impl OrgGetCurrent {
    /// Create a get-current-org operation.
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
    pub async fn run(&self) -> Result<OrgOutput, OperationError> {
        self.client.get_json("/api/org").await
    }
}

#[async_trait]
impl Operation for OrgGetCurrent {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }
}

impl TypedOperation for OrgGetCurrent {
    type Output = OrgOutput;
}

/// Update the current organization.
///
/// Sends a `PUT /api/org` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgUpdateCurrent;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgUpdateCurrent::new(&grafana, json!({"name": "New Org Name"}));
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgUpdateCurrent {
    client: GrafanaClient,
    body: Value,
}

impl OrgUpdateCurrent {
    /// Create an update-current-org operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            client: client.clone(),
            body,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client.put_json("/api/org", &self.body).await
    }
}

#[async_trait]
impl Operation for OrgUpdateCurrent {
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

/// Get users in the current organization.
///
/// Sends a `GET /api/org/users` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgGetCurrentUsers;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgGetCurrentUsers::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgGetCurrentUsers {
    client: GrafanaClient,
}

impl OrgGetCurrentUsers {
    /// Create a get-current-org-users operation.
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
    pub async fn run(&self) -> Result<Vec<OrgUserOutput>, OperationError> {
        self.client.get_json("/api/org/users").await
    }
}

#[async_trait]
impl Operation for OrgGetCurrentUsers {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }
}

impl TypedOperation for OrgGetCurrentUsers {
    type Output = Vec<OrgUserOutput>;
}

/// Add a user to the current organization.
///
/// Sends a `POST /api/org/users` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgAddCurrentUser;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgAddCurrentUser::new(&grafana, json!({"loginOrEmail": "user@ex.com", "role": "Viewer"}));
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgAddCurrentUser {
    client: GrafanaClient,
    body: Value,
}

impl OrgAddCurrentUser {
    /// Create an add-current-org-user operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            client: client.clone(),
            body,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client.post_json("/api/org/users", &self.body).await
    }
}

#[async_trait]
impl Operation for OrgAddCurrentUser {
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

/// Update a user's role in the current organization.
///
/// Sends a `PATCH /api/org/users/{user_id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgUpdateCurrentUser;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgUpdateCurrentUser::new(&grafana, 5, json!({"role": "Editor"}));
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgUpdateCurrentUser {
    client: GrafanaClient,
    user_id: u64,
    body: Value,
}

impl OrgUpdateCurrentUser {
    /// Create an update-current-org-user operation.
    pub fn new(client: &GrafanaClient, user_id: u64, body: Value) -> Self {
        Self {
            client: client.clone(),
            user_id,
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
            .patch_json(&format!("/api/org/users/{}", self.user_id), &self.body)
            .await
    }
}

#[async_trait]
impl Operation for OrgUpdateCurrentUser {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "user_id": self.user_id, "body": self.body }))
    }
}

/// Remove a user from the current organization.
///
/// Sends a `DELETE /api/org/users/{user_id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::organizations::OrgRemoveCurrentUser;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = OrgRemoveCurrentUser::new(&grafana, 5);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct OrgRemoveCurrentUser {
    client: GrafanaClient,
    user_id: u64,
}

impl OrgRemoveCurrentUser {
    /// Create a remove-current-org-user operation.
    pub fn new(client: &GrafanaClient, user_id: u64) -> Self {
        Self {
            client: client.clone(),
            user_id,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .delete_json(&format!("/api/org/users/{}", self.user_id))
            .await
    }
}

#[async_trait]
impl Operation for OrgRemoveCurrentUser {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "user_id": self.user_id }))
    }
}
