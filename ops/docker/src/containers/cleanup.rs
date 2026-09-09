//! Container cleanup operations: stats, changes, prune.

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::Docker;
use bollard::query_parameters::{PruneContainersOptions, StatsOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_stream::StreamExt;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// ContainerStats
// ---------------------------------------------------------------------------

/// Output of a single stats snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStatsOutput {
    /// The stats data as JSON.
    pub data: Value,
}

/// Get a single stats snapshot from a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerStats;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerStats::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerStats {
    docker: Docker,
    container: String,
}

impl ContainerStats {
    /// Create a new container-stats operation (single snapshot).
    pub fn new(client: impl Into<DockerRef>, container: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the container does not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ContainerStatsOutput, OperationError> {
        let options = StatsOptions {
            stream: false,
            one_shot: true,
        };
        let mut stream = self.docker.stats(&self.container, Some(options));
        let stats = stream
            .next()
            .await
            .ok_or_else(|| OperationError::External {
                origin: "docker".to_string(),
                message: "no stats returned".to_string(),
            })?
            .map_err(docker_error)?;
        let data = serde_json::to_value(&stats).map_err(|e| OperationError::External {
            origin: "docker".to_string(),
            message: e.to_string(),
        })?;
        Ok(ContainerStatsOutput { data })
    }
}

#[async_trait]
impl Operation for ContainerStats {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_stats",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerStats {
    type Output = ContainerStatsOutput;
}

// ---------------------------------------------------------------------------
// ContainerChanges
// ---------------------------------------------------------------------------

/// A single filesystem change in a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerChange {
    /// The file path.
    pub path: String,
    /// The kind of change (0=Modified, 1=Added, 2=Deleted).
    pub kind: i32,
}

/// Output of listing container filesystem changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerChangesOutput {
    /// The filesystem changes.
    pub changes: Vec<ContainerChange>,
}

/// List filesystem changes in a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerChanges;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerChanges::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerChanges {
    docker: Docker,
    container: String,
}

impl ContainerChanges {
    /// Create a new container-changes operation.
    pub fn new(client: impl Into<DockerRef>, container: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the container does not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ContainerChangesOutput, OperationError> {
        let response = self
            .docker
            .container_changes(&self.container)
            .await
            .map_err(docker_error)?;
        let changes = response
            .unwrap_or_default()
            .into_iter()
            .map(|c| ContainerChange {
                path: c.path,
                kind: c.kind as i32,
            })
            .collect();
        Ok(ContainerChangesOutput { changes })
    }
}

#[async_trait]
impl Operation for ContainerChanges {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_changes",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerChanges {
    type Output = ContainerChangesOutput;
}

// ---------------------------------------------------------------------------
// ContainerPrune
// ---------------------------------------------------------------------------

/// Output of pruning stopped containers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPruneOutput {
    /// IDs of removed containers.
    pub containers_deleted: Vec<String>,
    /// Disk space reclaimed in bytes.
    pub space_reclaimed: u64,
}

/// Remove all stopped containers.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerPrune;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerPrune::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerPrune {
    docker: Docker,
    filters: HashMap<String, Vec<String>>,
}

impl ContainerPrune {
    /// Create a new container-prune operation.
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
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ContainerPruneOutput, OperationError> {
        let options = PruneContainersOptions {
            filters: Some(self.filters.clone()),
        };
        let response = self
            .docker
            .prune_containers(Some(options))
            .await
            .map_err(docker_error)?;
        Ok(ContainerPruneOutput {
            containers_deleted: response.containers_deleted.unwrap_or_default(),
            space_reclaimed: response.space_reclaimed.unwrap_or(0) as u64,
        })
    }
}

#[async_trait]
impl Operation for ContainerPrune {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_prune",
        }))
    }
}

impl TypedOperation for ContainerPrune {
    type Output = ContainerPruneOutput;
}
