//! Contact point operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::types::ContactPointOutput;

/// List all contact points.
///
/// Sends a `GET /api/v1/provisioning/contact-points` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::ContactPointList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ContactPointList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ContactPointList {
    client: GrafanaClient,
}

impl ContactPointList {
    /// Create a list-contact-points operation.
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
    pub async fn run(&self) -> Result<Vec<ContactPointOutput>, OperationError> {
        self.client
            .get_json("/api/v1/provisioning/contact-points")
            .await
    }
}

#[async_trait]
impl Operation for ContactPointList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }
}

impl TypedOperation for ContactPointList {
    type Output = Vec<ContactPointOutput>;
}

/// Create a contact point.
///
/// Sends a `POST /api/v1/provisioning/contact-points` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::ContactPointCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "email-ops", "type": "email", "settings": {}});
/// let op = ContactPointCreate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ContactPointCreate {
    client: GrafanaClient,
    body: Value,
}

impl ContactPointCreate {
    /// Create a create-contact-point operation.
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
    pub async fn run(&self) -> Result<ContactPointOutput, OperationError> {
        self.client
            .post_json("/api/v1/provisioning/contact-points", &self.body)
            .await
    }
}

#[async_trait]
impl Operation for ContactPointCreate {
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

impl TypedOperation for ContactPointCreate {
    type Output = ContactPointOutput;
}

/// Update a contact point.
///
/// Sends a `PUT /api/v1/provisioning/contact-points/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::ContactPointUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "email-ops-v2", "type": "email", "settings": {}});
/// let op = ContactPointUpdate::new(&grafana, "cp-uid", body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ContactPointUpdate {
    client: GrafanaClient,
    uid: String,
    body: Value,
}

impl ContactPointUpdate {
    /// Create an update-contact-point operation.
    pub fn new(client: &GrafanaClient, uid: &str, body: Value) -> Self {
        Self {
            client: client.clone(),
            uid: uid.to_string(),
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
            .put_json(
                &format!("/api/v1/provisioning/contact-points/{}", self.uid),
                &self.body,
            )
            .await
    }
}

#[async_trait]
impl Operation for ContactPointUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid, "body": self.body }))
    }
}

/// Delete a contact point.
///
/// Sends a `DELETE /api/v1/provisioning/contact-points/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::ContactPointDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ContactPointDelete::new(&grafana, "cp-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ContactPointDelete {
    client: GrafanaClient,
    uid: String,
}

impl ContactPointDelete {
    /// Create a delete-contact-point operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            client: client.clone(),
            uid: uid.to_string(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .delete_json(&format!("/api/v1/provisioning/contact-points/{}", self.uid))
            .await
    }
}

#[async_trait]
impl Operation for ContactPointDelete {
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
