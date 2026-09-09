//! Volume operations: create, inspect, list, remove, prune.

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::Docker;
use bollard::models::VolumeCreateRequest;
use bollard::query_parameters::{ListVolumesOptions, PruneVolumesOptions, RemoveVolumeOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// VolumeCreate
// ---------------------------------------------------------------------------

/// Output of a volume creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeCreateOutput {
    /// The volume name.
    pub name: String,
    /// The mount point.
    pub mountpoint: String,
}

/// Create a volume.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::volumes::VolumeCreate;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = VolumeCreate::new(&client, "my-volume");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct VolumeCreate {
    docker: Docker,
    name: String,
    driver: Option<String>,
    labels: HashMap<String, String>,
}

impl VolumeCreate {
    /// Create a new volume-create operation.
    pub fn new(client: impl Into<DockerRef>, name: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            name: name.into(),
            driver: None,
            labels: HashMap::new(),
        }
    }

    /// Set the volume driver.
    pub fn driver(mut self, driver: impl Into<String>) -> Self {
        self.driver = Some(driver.into());
        self
    }

    /// Add a label.
    pub fn label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the volume cannot be created.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<VolumeCreateOutput, OperationError> {
        let request = VolumeCreateRequest {
            name: Some(self.name.clone()),
            driver: Some(self.driver.clone().unwrap_or_else(|| "local".to_string())),
            labels: Some(self.labels.clone()),
            ..Default::default()
        };
        let response = self
            .docker
            .create_volume(request)
            .await
            .map_err(docker_error)?;
        Ok(VolumeCreateOutput {
            name: response.name,
            mountpoint: response.mountpoint,
        })
    }
}

#[async_trait]
impl Operation for VolumeCreate {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "volume_create",
            "name": self.name,
        }))
    }
}

impl TypedOperation for VolumeCreate {
    type Output = VolumeCreateOutput;
}

// ---------------------------------------------------------------------------
// VolumeInspect
// ---------------------------------------------------------------------------

/// Output of a volume inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeInspectOutput {
    /// The volume name.
    pub name: String,
    /// The mount point.
    pub mountpoint: String,
    /// The driver.
    pub driver: String,
}

/// Inspect a volume.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::volumes::VolumeInspect;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = VolumeInspect::new(&client, "my-volume");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct VolumeInspect {
    docker: Docker,
    name: String,
}

impl VolumeInspect {
    /// Create a new volume-inspect operation.
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
    /// Returns [`OperationError::External`] if the volume does not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<VolumeInspectOutput, OperationError> {
        let response = self
            .docker
            .inspect_volume(&self.name)
            .await
            .map_err(docker_error)?;
        Ok(VolumeInspectOutput {
            name: response.name,
            mountpoint: response.mountpoint,
            driver: response.driver,
        })
    }
}

#[async_trait]
impl Operation for VolumeInspect {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "volume_inspect",
            "name": self.name,
        }))
    }
}

impl TypedOperation for VolumeInspect {
    type Output = VolumeInspectOutput;
}

// ---------------------------------------------------------------------------
// VolumeList
// ---------------------------------------------------------------------------

/// A single entry in the volume list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeListEntry {
    /// Volume name.
    pub name: String,
    /// Volume driver.
    pub driver: String,
    /// Mount point.
    pub mountpoint: String,
}

/// Output of a volume list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeListOutput {
    /// The volumes.
    pub volumes: Vec<VolumeListEntry>,
}

/// List volumes.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::volumes::VolumeList;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = VolumeList::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct VolumeList {
    docker: Docker,
    filters: HashMap<String, Vec<String>>,
}

impl VolumeList {
    /// Create a new volume-list operation.
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
    pub async fn run(&self, _ctx: &OperationContext) -> Result<VolumeListOutput, OperationError> {
        let options = ListVolumesOptions {
            filters: Some(self.filters.clone()),
        };
        let response = self
            .docker
            .list_volumes(Some(options))
            .await
            .map_err(docker_error)?;
        let volumes = response
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| VolumeListEntry {
                name: v.name,
                driver: v.driver,
                mountpoint: v.mountpoint,
            })
            .collect();
        Ok(VolumeListOutput { volumes })
    }
}

#[async_trait]
impl Operation for VolumeList {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "volume_list",
        }))
    }
}

impl TypedOperation for VolumeList {
    type Output = VolumeListOutput;
}

// ---------------------------------------------------------------------------
// VolumeRemove
// ---------------------------------------------------------------------------

/// Output of a volume removal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeRemoveOutput {
    /// The removed volume name.
    pub name: String,
}

/// Remove a volume.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::volumes::VolumeRemove;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = VolumeRemove::new(&client, "my-volume");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct VolumeRemove {
    docker: Docker,
    name: String,
    force: bool,
}

impl VolumeRemove {
    /// Create a new volume-remove operation.
    pub fn new(client: impl Into<DockerRef>, name: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            name: name.into(),
            force: false,
        }
    }

    /// Force-remove the volume.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the volume does not exist.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<VolumeRemoveOutput, OperationError> {
        self.docker
            .remove_volume(&self.name, Some(RemoveVolumeOptions { force: self.force }))
            .await
            .map_err(docker_error)?;
        Ok(VolumeRemoveOutput {
            name: self.name.clone(),
        })
    }
}

#[async_trait]
impl Operation for VolumeRemove {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "volume_remove",
            "name": self.name,
        }))
    }
}

impl TypedOperation for VolumeRemove {
    type Output = VolumeRemoveOutput;
}

// ---------------------------------------------------------------------------
// VolumePrune
// ---------------------------------------------------------------------------

/// Output of pruning unused volumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumePruneOutput {
    /// Names of removed volumes.
    pub volumes_deleted: Vec<String>,
    /// Disk space reclaimed in bytes.
    pub space_reclaimed: u64,
}

/// Remove unused volumes.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::volumes::VolumePrune;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = VolumePrune::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct VolumePrune {
    docker: Docker,
    filters: HashMap<String, Vec<String>>,
}

impl VolumePrune {
    /// Create a new volume-prune operation.
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
    pub async fn run(&self, _ctx: &OperationContext) -> Result<VolumePruneOutput, OperationError> {
        let options = PruneVolumesOptions {
            filters: Some(self.filters.clone()),
        };
        let response = self
            .docker
            .prune_volumes(Some(options))
            .await
            .map_err(docker_error)?;
        Ok(VolumePruneOutput {
            volumes_deleted: response.volumes_deleted.unwrap_or_default(),
            space_reclaimed: response.space_reclaimed.unwrap_or(0) as u64,
        })
    }
}

#[async_trait]
impl Operation for VolumePrune {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "volume_prune",
        }))
    }
}

impl TypedOperation for VolumePrune {
    type Output = VolumePruneOutput;
}
