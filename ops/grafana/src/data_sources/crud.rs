//! Data source CRUD and query operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::GrafanaClient;

use super::DataSourceOutput;

/// Create a data source.
///
/// Sends a `POST /api/datasources` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::data_sources::DataSourceCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "Prometheus", "type": "prometheus", "url": "http://prom:9090"});
/// let op = DataSourceCreate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DataSourceCreate {
    client: GrafanaClient,
    body: Value,
}

impl DataSourceCreate {
    /// Create a new data-source creation operation.
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
    pub async fn run(&self) -> Result<DataSourceOutput, OperationError> {
        self.client.post_json("/api/datasources", &self.body).await
    }
}

#[async_trait]
impl Operation for DataSourceCreate {
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

impl TypedOperation for DataSourceCreate {
    type Output = DataSourceOutput;
}

/// List all data sources.
///
/// Sends a `GET /api/datasources` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::data_sources::DataSourceList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DataSourceList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DataSourceList {
    client: GrafanaClient,
}

impl DataSourceList {
    /// Create a list-data-sources operation.
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
    pub async fn run(&self) -> Result<Vec<DataSourceOutput>, OperationError> {
        self.client.get_json("/api/datasources").await
    }
}

#[async_trait]
impl Operation for DataSourceList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        GrafanaClient::to_value(&self.run().await?)
    }
}

impl TypedOperation for DataSourceList {
    type Output = Vec<DataSourceOutput>;
}

/// Update a data source by numeric ID.
///
/// Sends a `PUT /api/datasources/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::data_sources::DataSourceUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "Prometheus", "type": "prometheus", "url": "http://prom:9090"});
/// let op = DataSourceUpdate::new(&grafana, 1, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DataSourceUpdate {
    client: GrafanaClient,
    id: u64,
    body: Value,
}

impl DataSourceUpdate {
    /// Create an update-data-source operation.
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
    pub async fn run(&self) -> Result<DataSourceOutput, OperationError> {
        self.client
            .put_json(&format!("/api/datasources/{}", self.id), &self.body)
            .await
    }
}

#[async_trait]
impl Operation for DataSourceUpdate {
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

impl TypedOperation for DataSourceUpdate {
    type Output = DataSourceOutput;
}

/// Delete a data source by numeric ID.
///
/// Sends a `DELETE /api/datasources/{id}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::data_sources::DataSourceDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DataSourceDelete::new(&grafana, 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DataSourceDelete {
    client: GrafanaClient,
    id: u64,
}

impl DataSourceDelete {
    /// Create a delete-data-source operation.
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
            .delete_json(&format!("/api/datasources/{}", self.id))
            .await
    }
}

#[async_trait]
impl Operation for DataSourceDelete {
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

/// Query a data source.
///
/// Sends a `POST /api/ds/query` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::data_sources::DataSourceQuery;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"queries": [{"refId": "A", "datasourceId": 1}]});
/// let op = DataSourceQuery::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DataSourceQuery {
    client: GrafanaClient,
    body: Value,
}

impl DataSourceQuery {
    /// Create a data-source query operation.
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
        self.client.post_json("/api/ds/query", &self.body).await
    }
}

#[async_trait]
impl Operation for DataSourceQuery {
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
