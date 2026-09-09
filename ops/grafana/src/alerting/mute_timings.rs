//! Mute timing operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{delete, get, post, put, to_value};

use super::types::MuteTimingOutput;

/// List all mute timings.
///
/// Sends a `GET /api/v1/provisioning/mute-timings` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::MuteTimingList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = MuteTimingList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct MuteTimingList {
    url: String,
    token: String,
    http: reqwest::Client,
}

impl MuteTimingList {
    /// Create a list-mute-timings operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            url: client.url("/api/v1/provisioning/mute-timings"),
            token: client.token().to_string(),
            http: client.http().clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<MuteTimingOutput>, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for MuteTimingList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }
}

impl TypedOperation for MuteTimingList {
    type Output = Vec<MuteTimingOutput>;
}

/// Create a mute timing.
///
/// Sends a `POST /api/v1/provisioning/mute-timings` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::MuteTimingCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "weekends", "time_intervals": []});
/// let op = MuteTimingCreate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct MuteTimingCreate {
    url: String,
    token: String,
    http: reqwest::Client,
    body: Value,
}

impl MuteTimingCreate {
    /// Create a create-mute-timing operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            url: client.url("/api/v1/provisioning/mute-timings"),
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
    pub async fn run(&self) -> Result<MuteTimingOutput, OperationError> {
        post(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for MuteTimingCreate {
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

impl TypedOperation for MuteTimingCreate {
    type Output = MuteTimingOutput;
}

/// Update a mute timing.
///
/// Sends a `PUT /api/v1/provisioning/mute-timings/{name}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::MuteTimingUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "weekends", "time_intervals": []});
/// let op = MuteTimingUpdate::new(&grafana, "weekends", body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct MuteTimingUpdate {
    url: String,
    token: String,
    http: reqwest::Client,
    name: String,
    body: Value,
}

impl MuteTimingUpdate {
    /// Create an update-mute-timing operation.
    pub fn new(client: &GrafanaClient, name: &str, body: Value) -> Self {
        Self {
            url: client.url(&format!("/api/v1/provisioning/mute-timings/{name}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            name: name.to_string(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<MuteTimingOutput, OperationError> {
        put(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for MuteTimingUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "name": self.name, "body": self.body }))
    }
}

impl TypedOperation for MuteTimingUpdate {
    type Output = MuteTimingOutput;
}

/// Delete a mute timing.
///
/// Sends a `DELETE /api/v1/provisioning/mute-timings/{name}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::alerting::MuteTimingDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = MuteTimingDelete::new(&grafana, "weekends");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct MuteTimingDelete {
    url: String,
    token: String,
    http: reqwest::Client,
    name: String,
}

impl MuteTimingDelete {
    /// Create a delete-mute-timing operation.
    pub fn new(client: &GrafanaClient, name: &str) -> Self {
        Self {
            url: client.url(&format!("/api/v1/provisioning/mute-timings/{name}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            name: name.to_string(),
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
impl Operation for MuteTimingDelete {
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
