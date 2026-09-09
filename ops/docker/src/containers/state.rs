//! Container state operations: pause, unpause, rename, top.

use async_trait::async_trait;
use bollard::Docker;
use bollard::query_parameters::{RenameContainerOptions, TopOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// ContainerPause
// ---------------------------------------------------------------------------

/// Output of a container pause.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPauseOutput {
    /// The container ID or name.
    pub container: String,
}

/// Pause a running container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerPause;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerPause::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerPause {
    docker: Docker,
    container: String,
}

impl ContainerPause {
    /// Create a new container-pause operation.
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
    ) -> Result<ContainerPauseOutput, OperationError> {
        self.docker
            .pause_container(&self.container)
            .await
            .map_err(docker_error)?;
        Ok(ContainerPauseOutput {
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for ContainerPause {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_pause",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerPause {
    type Output = ContainerPauseOutput;
}

// ---------------------------------------------------------------------------
// ContainerUnpause
// ---------------------------------------------------------------------------

/// Output of a container unpause.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerUnpauseOutput {
    /// The container ID or name.
    pub container: String,
}

/// Unpause a paused container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerUnpause;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerUnpause::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerUnpause {
    docker: Docker,
    container: String,
}

impl ContainerUnpause {
    /// Create a new container-unpause operation.
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
    ) -> Result<ContainerUnpauseOutput, OperationError> {
        self.docker
            .unpause_container(&self.container)
            .await
            .map_err(docker_error)?;
        Ok(ContainerUnpauseOutput {
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for ContainerUnpause {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_unpause",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerUnpause {
    type Output = ContainerUnpauseOutput;
}

// ---------------------------------------------------------------------------
// ContainerRename
// ---------------------------------------------------------------------------

/// Output of a container rename.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRenameOutput {
    /// The old container name.
    pub old_name: String,
    /// The new container name.
    pub new_name: String,
}

/// Rename a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerRename;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerRename::new(&client, "old-name", "new-name");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerRename {
    docker: Docker,
    container: String,
    new_name: String,
}

impl ContainerRename {
    /// Create a new container-rename operation.
    pub fn new(
        client: impl Into<DockerRef>,
        container: impl Into<String>,
        new_name: impl Into<String>,
    ) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
            new_name: new_name.into(),
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
    ) -> Result<ContainerRenameOutput, OperationError> {
        let options = RenameContainerOptions {
            name: self.new_name.clone(),
        };
        self.docker
            .rename_container(&self.container, options)
            .await
            .map_err(docker_error)?;
        Ok(ContainerRenameOutput {
            old_name: self.container.clone(),
            new_name: self.new_name.clone(),
        })
    }
}

#[async_trait]
impl Operation for ContainerRename {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_rename",
            "container": self.container,
            "new_name": self.new_name,
        }))
    }
}

impl TypedOperation for ContainerRename {
    type Output = ContainerRenameOutput;
}

// ---------------------------------------------------------------------------
// ContainerTop
// ---------------------------------------------------------------------------

/// Output of a container top (process list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerTopOutput {
    /// Column titles.
    pub titles: Vec<String>,
    /// Process rows.
    pub processes: Vec<Vec<String>>,
}

/// List processes running inside a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerTop;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerTop::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerTop {
    docker: Docker,
    container: String,
    ps_args: Option<String>,
}

impl ContainerTop {
    /// Create a new container-top operation.
    pub fn new(client: impl Into<DockerRef>, container: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
            ps_args: None,
        }
    }

    /// Set custom ps arguments.
    pub fn ps_args(mut self, args: impl Into<String>) -> Self {
        self.ps_args = Some(args.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the container does not exist or
    /// is not running.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ContainerTopOutput, OperationError> {
        let options = self.ps_args.as_ref().map(|args| TopOptions {
            ps_args: args.to_string(),
        });
        let response = self
            .docker
            .top_processes(&self.container, options)
            .await
            .map_err(docker_error)?;
        Ok(ContainerTopOutput {
            titles: response.titles.unwrap_or_default(),
            processes: response.processes.unwrap_or_default(),
        })
    }
}

#[async_trait]
impl Operation for ContainerTop {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_top",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerTop {
    type Output = ContainerTopOutput;
}
