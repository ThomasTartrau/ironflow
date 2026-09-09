//! Network management: create, inspect, list, remove, prune.

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::Docker;
use bollard::models::NetworkCreateRequest;
use bollard::query_parameters::{InspectNetworkOptions, ListNetworksOptions, PruneNetworksOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// NetworkCreate
// ---------------------------------------------------------------------------

/// Output of a network creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkCreateOutput {
    /// The created network ID.
    pub id: String,
}

/// Create a network.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::networks::NetworkCreate;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = NetworkCreate::new(&client, "my-network");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct NetworkCreate {
    docker: Docker,
    name: String,
    driver: Option<String>,
}

impl NetworkCreate {
    /// Create a new network-create operation.
    pub fn new(client: impl Into<DockerRef>, name: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            name: name.into(),
            driver: None,
        }
    }

    /// Set the network driver (defaults to "bridge").
    pub fn driver(mut self, driver: impl Into<String>) -> Self {
        self.driver = Some(driver.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the network cannot be created.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<NetworkCreateOutput, OperationError> {
        let request = NetworkCreateRequest {
            name: self.name.clone(),
            driver: Some(self.driver.clone().unwrap_or_else(|| "bridge".to_string())),
            ..Default::default()
        };
        let response = self
            .docker
            .create_network(request)
            .await
            .map_err(docker_error)?;
        Ok(NetworkCreateOutput { id: response.id })
    }
}

#[async_trait]
impl Operation for NetworkCreate {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "network_create",
            "name": self.name,
        }))
    }
}

impl TypedOperation for NetworkCreate {
    type Output = NetworkCreateOutput;
}

// ---------------------------------------------------------------------------
// NetworkInspect
// ---------------------------------------------------------------------------

/// Output of a network inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInspectOutput {
    /// The full inspection response as JSON.
    pub data: Value,
}

/// Inspect a network.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::networks::NetworkInspect;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = NetworkInspect::new(&client, "my-network");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct NetworkInspect {
    docker: Docker,
    name: String,
}

impl NetworkInspect {
    /// Create a new network-inspect operation.
    pub fn new(client: impl Into<DockerRef>, name: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the network does not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<NetworkInspectOutput, OperationError> {
        let options = InspectNetworkOptions {
            verbose: false,
            scope: Some("local".to_string()),
        };
        let response = self
            .docker
            .inspect_network(&self.name, Some(options))
            .await
            .map_err(docker_error)?;
        let data = serde_json::to_value(&response).map_err(|e| OperationError::External {
            origin: "docker".to_string(),
            message: e.to_string(),
        })?;
        Ok(NetworkInspectOutput { data })
    }
}

#[async_trait]
impl Operation for NetworkInspect {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "network_inspect",
            "name": self.name,
        }))
    }
}

impl TypedOperation for NetworkInspect {
    type Output = NetworkInspectOutput;
}

// ---------------------------------------------------------------------------
// NetworkList
// ---------------------------------------------------------------------------

/// A single entry in the network list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkListEntry {
    /// Network ID.
    pub id: String,
    /// Network name.
    pub name: String,
    /// Network driver.
    pub driver: String,
}

/// Output of a network list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkListOutput {
    /// The networks.
    pub networks: Vec<NetworkListEntry>,
}

/// List networks.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::networks::NetworkList;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = NetworkList::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct NetworkList {
    docker: Docker,
    filters: HashMap<String, Vec<String>>,
}

impl NetworkList {
    /// Create a new network-list operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
            filters: HashMap::new(),
        }
    }

    /// Add a filter.
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters
            .entry(key.into())
            .or_default()
            .push(value.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the Docker daemon is
    /// unreachable.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<NetworkListOutput, OperationError> {
        let options = ListNetworksOptions {
            filters: Some(self.filters.clone()),
        };
        let networks = self
            .docker
            .list_networks(Some(options))
            .await
            .map_err(docker_error)?;
        let entries = networks
            .into_iter()
            .map(|n| NetworkListEntry {
                id: n.id.unwrap_or_default(),
                name: n.name.unwrap_or_default(),
                driver: n.driver.unwrap_or_default(),
            })
            .collect();
        Ok(NetworkListOutput { networks: entries })
    }
}

#[async_trait]
impl Operation for NetworkList {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "network_list",
        }))
    }
}

impl TypedOperation for NetworkList {
    type Output = NetworkListOutput;
}

// ---------------------------------------------------------------------------
// NetworkRemove
// ---------------------------------------------------------------------------

/// Output of a network removal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRemoveOutput {
    /// The removed network name.
    pub name: String,
}

/// Remove a network.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::networks::NetworkRemove;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = NetworkRemove::new(&client, "my-network");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct NetworkRemove {
    docker: Docker,
    name: String,
}

impl NetworkRemove {
    /// Create a new network-remove operation.
    pub fn new(client: impl Into<DockerRef>, name: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the network does not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<NetworkRemoveOutput, OperationError> {
        self.docker
            .remove_network(&self.name)
            .await
            .map_err(docker_error)?;
        Ok(NetworkRemoveOutput {
            name: self.name.clone(),
        })
    }
}

#[async_trait]
impl Operation for NetworkRemove {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "network_remove",
            "name": self.name,
        }))
    }
}

impl TypedOperation for NetworkRemove {
    type Output = NetworkRemoveOutput;
}

// ---------------------------------------------------------------------------
// NetworkPrune
// ---------------------------------------------------------------------------

/// Output of pruning unused networks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPruneOutput {
    /// Names of removed networks.
    pub networks_deleted: Vec<String>,
}

/// Remove unused networks.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::networks::NetworkPrune;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = NetworkPrune::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct NetworkPrune {
    docker: Docker,
    filters: HashMap<String, Vec<String>>,
}

impl NetworkPrune {
    /// Create a new network-prune operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
            filters: HashMap::new(),
        }
    }

    /// Add a filter.
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters
            .entry(key.into())
            .or_default()
            .push(value.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the Docker daemon is
    /// unreachable.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<NetworkPruneOutput, OperationError> {
        let options = PruneNetworksOptions {
            filters: Some(self.filters.clone()),
        };
        let response = self
            .docker
            .prune_networks(Some(options))
            .await
            .map_err(docker_error)?;
        Ok(NetworkPruneOutput {
            networks_deleted: response.networks_deleted.unwrap_or_default(),
        })
    }
}

#[async_trait]
impl Operation for NetworkPrune {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "network_prune",
        }))
    }
}

impl TypedOperation for NetworkPrune {
    type Output = NetworkPruneOutput;
}
