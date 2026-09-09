//! Team member operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::TeamMemberOutput;

/// Get members of a team.
///
/// Sends a `GET /api/teams/{id}/members` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::teams::TeamGetMembers;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = TeamGetMembers::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct TeamGetMembers {
    client: GrafanaClient,
    id: u64,
}

impl TeamGetMembers {
    /// Create a get-team-members operation.
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
    pub async fn run(&self) -> Result<Vec<TeamMemberOutput>, OperationError> {
        self.client
            .get_json(&format!("/api/teams/{}/members", self.id))
            .await
    }
}

#[async_trait]
impl Operation for TeamGetMembers {
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

impl TypedOperation for TeamGetMembers {
    type Output = Vec<TeamMemberOutput>;
}

/// Add a member to a team.
///
/// Sends a `POST /api/teams/{id}/members` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::teams::TeamAddMember;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = TeamAddMember::new(&grafana, 1, json!({"userId": 5}));
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct TeamAddMember {
    client: GrafanaClient,
    id: u64,
    body: Value,
}

impl TeamAddMember {
    /// Create an add-team-member operation.
    pub fn new(client: &GrafanaClient, id: u64, body: Value) -> Self {
        Self {
            client: client.clone(),
            id,
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
            .post_json(&format!("/api/teams/{}/members", self.id), &self.body)
            .await
    }
}

#[async_trait]
impl Operation for TeamAddMember {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "team_id": self.id, "body": self.body }))
    }
}

/// Remove a member from a team.
///
/// Sends a `DELETE /api/teams/{id}/members/{user_id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::teams::TeamRemoveMember;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = TeamRemoveMember::new(&grafana, 1, 5);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct TeamRemoveMember {
    client: GrafanaClient,
    id: u64,
    user_id: u64,
}

impl TeamRemoveMember {
    /// Create a remove-team-member operation.
    pub fn new(client: &GrafanaClient, id: u64, user_id: u64) -> Self {
        Self {
            client: client.clone(),
            id,
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
            .delete_json(&format!("/api/teams/{}/members/{}", self.id, self.user_id))
            .await
    }
}

#[async_trait]
impl Operation for TeamRemoveMember {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "team_id": self.id, "user_id": self.user_id }))
    }
}
