//! Service account operations.
//!
//! Provides CRUD, token management for Grafana service accounts
//! via the `/api/serviceaccounts/` endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;

/// Service account metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAccountOutput {
    /// Service account numeric ID.
    pub id: Option<u64>,
    /// Service account name.
    pub name: Option<String>,
    /// Service account login.
    pub login: Option<String>,
    /// Role (Viewer, Editor, Admin).
    pub role: Option<String>,
    /// Whether the service account is disabled.
    pub is_disabled: Option<bool>,
}

/// Token created for a service account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAccountTokenOutput {
    /// Token numeric ID.
    pub id: Option<u64>,
    /// Token name.
    pub name: Option<String>,
    /// Token key (only returned on creation).
    pub key: Option<String>,
}

/// Search service accounts.
///
/// Sends a `GET /api/serviceaccounts/search` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::service_accounts::ServiceAccountList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ServiceAccountList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ServiceAccountList {
    client: GrafanaClient,
}

impl ServiceAccountList {
    /// Create a list operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            client: client.clone(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client.get_json("/api/serviceaccounts/search").await
    }
}

#[async_trait]
impl Operation for ServiceAccountList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "endpoint": "serviceaccounts/search" }))
    }
}

/// Get a service account by ID.
///
/// Sends a `GET /api/serviceaccounts/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::service_accounts::ServiceAccountGet;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ServiceAccountGet::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ServiceAccountGet {
    client: GrafanaClient,
    id: u64,
}

impl ServiceAccountGet {
    /// Create a get operation.
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
    pub async fn run(&self) -> Result<ServiceAccountOutput, OperationError> {
        self.client
            .get_json(&format!("/api/serviceaccounts/{}", self.id))
            .await
    }
}

#[async_trait]
impl Operation for ServiceAccountGet {
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

impl TypedOperation for ServiceAccountGet {
    type Output = ServiceAccountOutput;
}

/// Create a service account.
///
/// Sends a `POST /api/serviceaccounts` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::service_accounts::ServiceAccountCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "my-sa", "role": "Viewer"});
/// let op = ServiceAccountCreate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ServiceAccountCreate {
    client: GrafanaClient,
    body: Value,
}

impl ServiceAccountCreate {
    /// Create a service account creation operation.
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
    pub async fn run(&self) -> Result<ServiceAccountOutput, OperationError> {
        self.client
            .post_json("/api/serviceaccounts", &self.body)
            .await
    }
}

#[async_trait]
impl Operation for ServiceAccountCreate {
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

impl TypedOperation for ServiceAccountCreate {
    type Output = ServiceAccountOutput;
}

/// Update a service account.
///
/// Sends a `PATCH /api/serviceaccounts/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::service_accounts::ServiceAccountUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "renamed-sa"});
/// let op = ServiceAccountUpdate::new(&grafana, 1, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ServiceAccountUpdate {
    client: GrafanaClient,
    id: u64,
    body: Value,
}

impl ServiceAccountUpdate {
    /// Create an update operation.
    pub fn new(client: &GrafanaClient, id: u64, body: Value) -> Self {
        Self {
            client: client.clone(),
            id,
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<ServiceAccountOutput, OperationError> {
        self.client
            .patch_json(&format!("/api/serviceaccounts/{}", self.id), &self.body)
            .await
    }
}

#[async_trait]
impl Operation for ServiceAccountUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "id": self.id, "body": self.body }))
    }
}

impl TypedOperation for ServiceAccountUpdate {
    type Output = ServiceAccountOutput;
}

/// Delete a service account.
///
/// Sends a `DELETE /api/serviceaccounts/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::service_accounts::ServiceAccountDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ServiceAccountDelete::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ServiceAccountDelete {
    client: GrafanaClient,
    id: u64,
}

impl ServiceAccountDelete {
    /// Create a delete operation.
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
            .delete_json(&format!("/api/serviceaccounts/{}", self.id))
            .await
    }
}

#[async_trait]
impl Operation for ServiceAccountDelete {
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

/// Create a token for a service account.
///
/// Sends a `POST /api/serviceaccounts/{id}/tokens` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::service_accounts::ServiceAccountCreateToken;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "my-token", "secondsToLive": 86400});
/// let op = ServiceAccountCreateToken::new(&grafana, 1, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ServiceAccountCreateToken {
    client: GrafanaClient,
    id: u64,
    body: Value,
}

impl ServiceAccountCreateToken {
    /// Create a token creation operation.
    pub fn new(client: &GrafanaClient, id: u64, body: Value) -> Self {
        Self {
            client: client.clone(),
            id,
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<ServiceAccountTokenOutput, OperationError> {
        self.client
            .post_json(
                &format!("/api/serviceaccounts/{}/tokens", self.id),
                &self.body,
            )
            .await
    }
}

#[async_trait]
impl Operation for ServiceAccountCreateToken {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut output = self.run().await?;
        // Redact the token key -- it is a one-time secret that must not be
        // persisted in the workflow step history.
        output.key = output.key.map(|_| "[REDACTED]".to_string());
        GrafanaClient::to_value(&output)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "id": self.id, "body": self.body }))
    }
}

impl TypedOperation for ServiceAccountCreateToken {
    type Output = ServiceAccountTokenOutput;
}

/// Delete a token from a service account.
///
/// Sends a `DELETE /api/serviceaccounts/{id}/tokens/{token_id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::service_accounts::ServiceAccountDeleteToken;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ServiceAccountDeleteToken::new(&grafana, 1, 42);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ServiceAccountDeleteToken {
    client: GrafanaClient,
    id: u64,
    token_id: u64,
}

impl ServiceAccountDeleteToken {
    /// Create a token deletion operation.
    pub fn new(client: &GrafanaClient, id: u64, token_id: u64) -> Self {
        Self {
            client: client.clone(),
            id,
            token_id,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .delete_json(&format!(
                "/api/serviceaccounts/{}/tokens/{}",
                self.id, self.token_id
            ))
            .await
    }
}

#[async_trait]
impl Operation for ServiceAccountDeleteToken {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "id": self.id, "token_id": self.token_id }))
    }
}
