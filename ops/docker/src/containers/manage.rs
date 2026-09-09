//! Container management: kill, remove, inspect, list.

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::Docker;
use bollard::query_parameters::{
    InspectContainerOptions, KillContainerOptions, ListContainersOptions, RemoveContainerOptions,
};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// ContainerKill
// ---------------------------------------------------------------------------

/// Output of a container kill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerKillOutput {
    /// The container ID or name.
    pub container: String,
}

/// Send a signal to a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerKill;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerKill::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerKill {
    docker: Docker,
    container: String,
    signal: Option<String>,
}

impl ContainerKill {
    /// Create a new container-kill operation.
    pub fn new(client: impl Into<DockerRef>, container: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
            signal: None,
        }
    }

    /// Set the signal to send (defaults to SIGKILL).
    pub fn signal(mut self, signal: impl Into<String>) -> Self {
        self.signal = Some(signal.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the container does not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ContainerKillOutput, OperationError> {
        let options = KillContainerOptions {
            signal: self.signal.clone().unwrap_or_else(|| "SIGKILL".to_string()),
        };
        self.docker
            .kill_container(&self.container, Some(options))
            .await
            .map_err(docker_error)?;
        Ok(ContainerKillOutput {
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for ContainerKill {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_kill",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerKill {
    type Output = ContainerKillOutput;
}

// ---------------------------------------------------------------------------
// ContainerRemove
// ---------------------------------------------------------------------------

/// Output of a container removal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRemoveOutput {
    /// The container ID or name that was removed.
    pub container: String,
}

/// Remove a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerRemove;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerRemove::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerRemove {
    docker: Docker,
    container: String,
    force: bool,
    volumes: bool,
}

impl ContainerRemove {
    /// Create a new container-remove operation.
    pub fn new(client: impl Into<DockerRef>, container: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
            force: false,
            volumes: false,
        }
    }

    /// Force-remove the container even if it is running.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }

    /// Also remove anonymous volumes associated with the container.
    pub fn volumes(mut self) -> Self {
        self.volumes = true;
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the container does not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ContainerRemoveOutput, OperationError> {
        let options = RemoveContainerOptions {
            force: self.force,
            v: self.volumes,
            ..Default::default()
        };
        self.docker
            .remove_container(&self.container, Some(options))
            .await
            .map_err(docker_error)?;
        Ok(ContainerRemoveOutput {
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for ContainerRemove {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_remove",
            "container": self.container,
            "force": self.force,
        }))
    }
}

impl TypedOperation for ContainerRemove {
    type Output = ContainerRemoveOutput;
}

// ---------------------------------------------------------------------------
// ContainerInspect
// ---------------------------------------------------------------------------

/// Output of a container inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInspectOutput {
    /// The full inspection response as JSON.
    pub data: Value,
}

/// Inspect a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerInspect;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerInspect::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerInspect {
    docker: Docker,
    container: String,
}

impl ContainerInspect {
    /// Create a new container-inspect operation.
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
    ) -> Result<ContainerInspectOutput, OperationError> {
        let options = InspectContainerOptions { size: false };
        let response = self
            .docker
            .inspect_container(&self.container, Some(options))
            .await
            .map_err(docker_error)?;
        let data = serde_json::to_value(&response).map_err(|e| OperationError::External {
            origin: "docker".to_string(),
            message: e.to_string(),
        })?;
        Ok(ContainerInspectOutput { data })
    }
}

#[async_trait]
impl Operation for ContainerInspect {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_inspect",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerInspect {
    type Output = ContainerInspectOutput;
}

// ---------------------------------------------------------------------------
// ContainerList
// ---------------------------------------------------------------------------

/// A single entry in the container list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerListEntry {
    /// Container ID.
    pub id: String,
    /// Container names.
    pub names: Vec<String>,
    /// Image name.
    pub image: String,
    /// Current state.
    pub state: String,
    /// Human-readable status string.
    pub status: String,
}

/// Output of a container list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerListOutput {
    /// The containers.
    pub containers: Vec<ContainerListEntry>,
}

/// List containers.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerList;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerList::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerList {
    docker: Docker,
    all: bool,
    filters: HashMap<String, Vec<String>>,
}

impl ContainerList {
    /// Create a new container-list operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
            all: false,
            filters: HashMap::new(),
        }
    }

    /// Include stopped containers.
    pub fn all(mut self) -> Self {
        self.all = true;
        self
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
    ) -> Result<ContainerListOutput, OperationError> {
        let options = ListContainersOptions {
            all: self.all,
            filters: Some(self.filters.clone()),
            ..Default::default()
        };
        let containers = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(docker_error)?;
        let entries = containers
            .into_iter()
            .map(|c| ContainerListEntry {
                id: c.id.unwrap_or_default(),
                names: c.names.unwrap_or_default(),
                image: c.image.unwrap_or_default(),
                state: c.state.map(|s| s.to_string()).unwrap_or_default(),
                status: c.status.unwrap_or_default(),
            })
            .collect();
        Ok(ContainerListOutput {
            containers: entries,
        })
    }
}

#[async_trait]
impl Operation for ContainerList {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_list",
            "all": self.all,
        }))
    }
}

impl TypedOperation for ContainerList {
    type Output = ContainerListOutput;
}
