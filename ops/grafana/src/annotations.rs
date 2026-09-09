//! Annotation operations.
//!
//! Provides CRUD, list, and tag operations for Grafana annotations
//! via the `/api/annotations/` endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;

/// Response from annotation creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationCreateOutput {
    /// Annotation numeric ID.
    pub id: Option<u64>,
    /// Status message.
    pub message: Option<String>,
}

/// An annotation entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationOutput {
    /// Annotation numeric ID.
    pub id: Option<u64>,
    /// Dashboard numeric ID.
    pub dashboard_id: Option<u64>,
    /// Panel numeric ID.
    pub panel_id: Option<u64>,
    /// Annotation text.
    pub text: Option<String>,
    /// Tags.
    pub tags: Option<Vec<String>>,
    /// Start time (epoch ms).
    pub time: Option<u64>,
    /// End time (epoch ms).
    pub time_end: Option<u64>,
}

/// Create an annotation.
///
/// Sends a `POST /api/annotations` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::annotations::AnnotationCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"text": "Deploy v1.2", "tags": ["deploy"]});
/// let op = AnnotationCreate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AnnotationCreate {
    client: GrafanaClient,
    body: Value,
}

impl AnnotationCreate {
    /// Create a new annotation operation.
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
    pub async fn run(&self) -> Result<AnnotationCreateOutput, OperationError> {
        self.client.post_json("/api/annotations", &self.body).await
    }
}

#[async_trait]
impl Operation for AnnotationCreate {
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

impl TypedOperation for AnnotationCreate {
    type Output = AnnotationCreateOutput;
}

/// List annotations with optional time-range filter.
///
/// Sends a `GET /api/annotations` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::annotations::AnnotationList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AnnotationList::new(&grafana, None, None);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AnnotationList {
    client: GrafanaClient,
    from: Option<u64>,
    to: Option<u64>,
}

impl AnnotationList {
    /// Create a list-annotations operation with optional time range.
    pub fn new(client: &GrafanaClient, from: Option<u64>, to: Option<u64>) -> Self {
        Self {
            client: client.clone(),
            from,
            to,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<AnnotationOutput>, OperationError> {
        let mut params = Vec::new();
        if let Some(f) = self.from {
            params.push(("from", f.to_string()));
        }
        if let Some(t) = self.to {
            params.push(("to", t.to_string()));
        }
        self.client
            .get_json_with_query("/api/annotations", &params)
            .await
    }
}

#[async_trait]
impl Operation for AnnotationList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "from": self.from, "to": self.to }))
    }
}

impl TypedOperation for AnnotationList {
    type Output = Vec<AnnotationOutput>;
}

/// Get an annotation by ID.
///
/// Sends a `GET /api/annotations/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::annotations::AnnotationGetById;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AnnotationGetById::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AnnotationGetById {
    client: GrafanaClient,
    id: u64,
}

impl AnnotationGetById {
    /// Create a get-annotation operation.
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
    pub async fn run(&self) -> Result<AnnotationOutput, OperationError> {
        self.client
            .get_json(&format!("/api/annotations/{}", self.id))
            .await
    }
}

#[async_trait]
impl Operation for AnnotationGetById {
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

impl TypedOperation for AnnotationGetById {
    type Output = AnnotationOutput;
}

/// Update an annotation (full replace).
///
/// Sends a `PUT /api/annotations/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::annotations::AnnotationUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"text": "Updated annotation"});
/// let op = AnnotationUpdate::new(&grafana, 1, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AnnotationUpdate {
    client: GrafanaClient,
    id: u64,
    body: Value,
}

impl AnnotationUpdate {
    /// Create an update-annotation operation.
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
            .put_json(&format!("/api/annotations/{}", self.id), &self.body)
            .await
    }
}

#[async_trait]
impl Operation for AnnotationUpdate {
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

/// Patch an annotation (partial update).
///
/// Sends a `PATCH /api/annotations/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::annotations::AnnotationPatch;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"text": "Patched"});
/// let op = AnnotationPatch::new(&grafana, 1, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AnnotationPatch {
    client: GrafanaClient,
    id: u64,
    body: Value,
}

impl AnnotationPatch {
    /// Create a patch-annotation operation.
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
            .patch_json(&format!("/api/annotations/{}", self.id), &self.body)
            .await
    }
}

#[async_trait]
impl Operation for AnnotationPatch {
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

/// Delete an annotation.
///
/// Sends a `DELETE /api/annotations/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::annotations::AnnotationDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AnnotationDelete::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AnnotationDelete {
    client: GrafanaClient,
    id: u64,
}

impl AnnotationDelete {
    /// Create a delete-annotation operation.
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
            .delete_json(&format!("/api/annotations/{}", self.id))
            .await
    }
}

#[async_trait]
impl Operation for AnnotationDelete {
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

/// Get annotation tags.
///
/// Sends a `GET /api/annotations/tags` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::annotations::AnnotationGetTags;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AnnotationGetTags::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AnnotationGetTags {
    client: GrafanaClient,
}

impl AnnotationGetTags {
    /// Create a get-tags operation.
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
        self.client.get_json("/api/annotations/tags").await
    }
}

#[async_trait]
impl Operation for AnnotationGetTags {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }
}
