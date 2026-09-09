//! Container lifecycle operations: start, stop, restart.

use async_trait::async_trait;
use bollard::Docker;
use bollard::query_parameters::{RestartContainerOptions, StopContainerOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// ContainerStart
// ---------------------------------------------------------------------------

/// Output of a container start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStartOutput {
    /// The container ID or name.
    pub container: String,
}

/// Start a stopped container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerStart;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerStart::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerStart {
    docker: Docker,
    container: String,
}

impl ContainerStart {
    /// Create a new container-start operation.
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
    /// Returns [`OperationError::External`] if the container does not exist or
    /// cannot be started.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ContainerStartOutput, OperationError> {
        self.docker
            .start_container(
                &self.container,
                None::<bollard::query_parameters::StartContainerOptions>,
            )
            .await
            .map_err(docker_error)?;
        Ok(ContainerStartOutput {
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for ContainerStart {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_start",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerStart {
    type Output = ContainerStartOutput;
}

// ---------------------------------------------------------------------------
// ContainerStop
// ---------------------------------------------------------------------------

/// Output of a container stop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStopOutput {
    /// The container ID or name.
    pub container: String,
}

/// Stop a running container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerStop;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerStop::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerStop {
    docker: Docker,
    container: String,
    timeout: Option<i64>,
}

impl ContainerStop {
    /// Create a new container-stop operation.
    pub fn new(client: impl Into<DockerRef>, container: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
            timeout: None,
        }
    }

    /// Set the timeout in seconds before killing the container.
    pub fn timeout(mut self, secs: i64) -> Self {
        self.timeout = Some(secs);
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
    ) -> Result<ContainerStopOutput, OperationError> {
        let options = StopContainerOptions {
            t: Some(self.timeout.unwrap_or(10) as i32),
            signal: None,
        };
        self.docker
            .stop_container(&self.container, Some(options))
            .await
            .map_err(docker_error)?;
        Ok(ContainerStopOutput {
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for ContainerStop {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_stop",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerStop {
    type Output = ContainerStopOutput;
}

// ---------------------------------------------------------------------------
// ContainerRestart
// ---------------------------------------------------------------------------

/// Output of a container restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRestartOutput {
    /// The container ID or name.
    pub container: String,
}

/// Restart a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerRestart;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerRestart::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerRestart {
    docker: Docker,
    container: String,
    timeout: Option<i64>,
}

impl ContainerRestart {
    /// Create a new container-restart operation.
    pub fn new(client: impl Into<DockerRef>, container: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
            timeout: None,
        }
    }

    /// Set the timeout in seconds before killing the container.
    pub fn timeout(mut self, secs: i64) -> Self {
        self.timeout = Some(secs);
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
    ) -> Result<ContainerRestartOutput, OperationError> {
        let options = RestartContainerOptions {
            t: Some(self.timeout.unwrap_or(10) as i32),
            signal: None,
        };
        self.docker
            .restart_container(&self.container, Some(options))
            .await
            .map_err(docker_error)?;
        Ok(ContainerRestartOutput {
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for ContainerRestart {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_restart",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerRestart {
    type Output = ContainerRestartOutput;
}
