//! Data source lookup operations (by ID, UID, name).

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{get, to_value};

use super::DataSourceOutput;

/// Get a data source by numeric ID.
///
/// Sends a `GET /api/datasources/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::data_sources::DataSourceGetById;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DataSourceGetById::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DataSourceGetById {
    url: String,
    token: String,
    http: reqwest::Client,
    id: u64,
}

impl DataSourceGetById {
    /// Create a get-data-source-by-id operation.
    pub fn new(client: &GrafanaClient, id: u64) -> Self {
        Self {
            url: client.url(&format!("/api/datasources/{id}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            id,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<DataSourceOutput, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for DataSourceGetById {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "id": self.id }))
    }
}

impl TypedOperation for DataSourceGetById {
    type Output = DataSourceOutput;
}

/// Get a data source by UID.
///
/// Sends a `GET /api/datasources/uid/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::data_sources::DataSourceGetByUid;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DataSourceGetByUid::new(&grafana, "ds-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DataSourceGetByUid {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl DataSourceGetByUid {
    /// Create a get-data-source-by-uid operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/datasources/uid/{uid}")),
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
    pub async fn run(&self) -> Result<DataSourceOutput, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for DataSourceGetByUid {
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

impl TypedOperation for DataSourceGetByUid {
    type Output = DataSourceOutput;
}

/// Get a data source by name.
///
/// Sends a `GET /api/datasources/name/{name}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::data_sources::DataSourceGetByName;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DataSourceGetByName::new(&grafana, "Prometheus");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DataSourceGetByName {
    url: String,
    token: String,
    http: reqwest::Client,
    name: String,
}

impl DataSourceGetByName {
    /// Create a get-data-source-by-name operation.
    pub fn new(client: &GrafanaClient, name: &str) -> Self {
        Self {
            url: client.url(&format!("/api/datasources/name/{name}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            name: name.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<DataSourceOutput, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for DataSourceGetByName {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "name": self.name }))
    }
}

impl TypedOperation for DataSourceGetByName {
    type Output = DataSourceOutput;
}
