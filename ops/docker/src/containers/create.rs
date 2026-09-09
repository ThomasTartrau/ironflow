//! Container creation.

use async_trait::async_trait;
use bollard::Docker;
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::CreateContainerOptions;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

/// Output of a container creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerCreateOutput {
    /// The created container ID.
    pub id: String,
    /// Warnings from the Docker daemon.
    pub warnings: Vec<String>,
}

/// Create a new container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerCreate;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerCreate::new(&client, "my-container", "alpine:latest");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerCreate {
    docker: Docker,
    name: String,
    image: String,
    cmd: Option<Vec<String>>,
    env: Option<Vec<String>>,
    exposed_ports: Option<Vec<String>>,
}

impl ContainerCreate {
    /// Create a new container-create operation.
    pub fn new(
        client: impl Into<DockerRef>,
        name: impl Into<String>,
        image: impl Into<String>,
    ) -> Self {
        Self {
            docker: client.into().0,
            name: name.into(),
            image: image.into(),
            cmd: None,
            env: None,
            exposed_ports: None,
        }
    }

    /// Set the command to run in the container.
    pub fn cmd(mut self, cmd: Vec<String>) -> Self {
        self.cmd = Some(cmd);
        self
    }

    /// Set environment variables for the container.
    pub fn env(mut self, env: Vec<String>) -> Self {
        self.env = Some(env);
        self
    }

    /// Set exposed ports for the container.
    pub fn exposed_ports(mut self, ports: Vec<String>) -> Self {
        self.exposed_ports = Some(ports);
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the Docker daemon rejects the
    /// creation request.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ContainerCreateOutput, OperationError> {
        let options = CreateContainerOptions {
            name: Some(self.name.clone()),
            platform: String::new(),
        };
        let config = ContainerCreateBody {
            image: Some(self.image.clone()),
            cmd: self.cmd.clone(),
            env: self.env.clone(),
            exposed_ports: self.exposed_ports.clone(),
            ..Default::default()
        };
        let response = self
            .docker
            .create_container(Some(options), config)
            .await
            .map_err(docker_error)?;
        Ok(ContainerCreateOutput {
            id: response.id,
            warnings: response.warnings,
        })
    }
}

#[async_trait]
impl Operation for ContainerCreate {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_create",
            "name": self.name,
            "image": self.image,
        }))
    }
}

impl TypedOperation for ContainerCreate {
    type Output = ContainerCreateOutput;
}
