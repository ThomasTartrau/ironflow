//! Notification template operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::TemplateOutput;

/// List all notification templates.
///
/// Sends a `GET /api/v1/provisioning/templates` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::TemplateList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = TemplateList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct TemplateList {
    client: GrafanaClient,
}

impl TemplateList {
    /// Create a list-templates operation.
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
    pub async fn run(&self) -> Result<Vec<TemplateOutput>, OperationError> {
        self.client.get_json("/api/v1/provisioning/templates").await
    }
}

#[async_trait]
impl Operation for TemplateList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }
}

impl TypedOperation for TemplateList {
    type Output = Vec<TemplateOutput>;
}

/// Create a notification template.
///
/// Sends a `PUT /api/v1/provisioning/templates/{name}` request (Grafana uses PUT for create).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::TemplateCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"template": "{{ define \"my-tmpl\" }}...{{ end }}"});
/// let op = TemplateCreate::new(&grafana, "my-tmpl", body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct TemplateCreate {
    client: GrafanaClient,
    name: String,
    body: Value,
}

impl TemplateCreate {
    /// Create a create-template operation.
    pub fn new(client: &GrafanaClient, name: &str, body: Value) -> Self {
        Self {
            client: client.clone(),
            name: name.to_string(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<TemplateOutput, OperationError> {
        self.client
            .put_json(
                &format!("/api/v1/provisioning/templates/{}", self.name),
                &self.body,
            )
            .await
    }
}

#[async_trait]
impl Operation for TemplateCreate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "name": self.name, "body": self.body }))
    }
}

impl TypedOperation for TemplateCreate {
    type Output = TemplateOutput;
}

/// Update a notification template.
///
/// Sends a `PUT /api/v1/provisioning/templates/{name}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::TemplateUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"template": "{{ define \"my-tmpl\" }}v2{{ end }}"});
/// let op = TemplateUpdate::new(&grafana, "my-tmpl", body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct TemplateUpdate {
    client: GrafanaClient,
    name: String,
    body: Value,
}

impl TemplateUpdate {
    /// Create an update-template operation.
    pub fn new(client: &GrafanaClient, name: &str, body: Value) -> Self {
        Self {
            client: client.clone(),
            name: name.to_string(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<TemplateOutput, OperationError> {
        self.client
            .put_json(
                &format!("/api/v1/provisioning/templates/{}", self.name),
                &self.body,
            )
            .await
    }
}

#[async_trait]
impl Operation for TemplateUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "name": self.name, "body": self.body }))
    }
}

impl TypedOperation for TemplateUpdate {
    type Output = TemplateOutput;
}

/// Delete a notification template.
///
/// Sends a `DELETE /api/v1/provisioning/templates/{name}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::TemplateDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = TemplateDelete::new(&grafana, "my-tmpl");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct TemplateDelete {
    client: GrafanaClient,
    name: String,
}

impl TemplateDelete {
    /// Create a delete-template operation.
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
            .delete_json(&format!("/api/v1/provisioning/templates/{}", self.name))
            .await
    }
}

#[async_trait]
impl Operation for TemplateDelete {
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
