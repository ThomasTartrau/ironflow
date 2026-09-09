//! User operations.
//!
//! Provides listing, lookup, search, and update for Grafana users
//! via the `/api/users/` and `/api/org/users` endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use ironflow_ops_common::helpers::{check_response_json, reqwest_err};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;

/// User metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserOutput {
    /// User numeric ID.
    pub id: Option<u64>,
    /// User login.
    pub login: Option<String>,
    /// User email.
    pub email: Option<String>,
    /// User display name.
    pub name: Option<String>,
    /// Whether the user is a server admin.
    pub is_admin: Option<bool>,
}

/// Paginated user search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSearchOutput {
    /// Users on this page.
    pub users: Option<Vec<UserOutput>>,
    /// Total count across all pages.
    pub total_count: Option<u64>,
    /// Current page number.
    pub page: Option<u64>,
    /// Results per page.
    pub per_page: Option<u64>,
}

/// List users in the current organization.
///
/// Sends a `GET /api/org/users` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::users::UserList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = UserList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct UserList {
    client: GrafanaClient,
}

impl UserList {
    /// Create a list-users operation.
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
    pub async fn run(&self) -> Result<Vec<UserOutput>, OperationError> {
        self.client.get_json("/api/org/users").await
    }
}

#[async_trait]
impl Operation for UserList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }
}

impl TypedOperation for UserList {
    type Output = Vec<UserOutput>;
}

/// Get a user by ID.
///
/// Sends a `GET /api/users/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::users::UserGetById;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = UserGetById::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct UserGetById {
    client: GrafanaClient,
    id: u64,
}

impl UserGetById {
    /// Create a get-user operation.
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
    pub async fn run(&self) -> Result<UserOutput, OperationError> {
        self.client
            .get_json(&format!("/api/users/{}", self.id))
            .await
    }
}

#[async_trait]
impl Operation for UserGetById {
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

impl TypedOperation for UserGetById {
    type Output = UserOutput;
}

/// Search users.
///
/// Sends a `GET /api/users/search?query={query}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::users::UserSearch;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = UserSearch::new(&grafana, "admin");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct UserSearch {
    client: GrafanaClient,
    query: String,
}

impl UserSearch {
    /// Create a search-users operation.
    pub fn new(client: &GrafanaClient, query: &str) -> Self {
        Self {
            client: client.clone(),
            query: query.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<UserSearchOutput, OperationError> {
        let resp = self
            .client
            .get_request("/api/users/search")
            .query(&[("query", &self.query)])
            .send()
            .await
            .map_err(reqwest_err)?;
        check_response_json(resp).await
    }
}

#[async_trait]
impl Operation for UserSearch {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "query": self.query }))
    }
}

impl TypedOperation for UserSearch {
    type Output = UserSearchOutput;
}

/// Update a user.
///
/// Sends a `PUT /api/users/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::users::UserUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = UserUpdate::new(&grafana, 1, json!({"name": "New Name"}));
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct UserUpdate {
    client: GrafanaClient,
    id: u64,
    body: Value,
}

impl UserUpdate {
    /// Create an update-user operation.
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
            .put_json(&format!("/api/users/{}", self.id), &self.body)
            .await
    }
}

#[async_trait]
impl Operation for UserUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "id": self.id, "body": self.body }))
    }
}
