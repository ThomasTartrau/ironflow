//! Network connection operations: connect, disconnect.

use async_trait::async_trait;
use bollard::Docker;
use bollard::models::{NetworkConnectRequest, NetworkDisconnectRequest};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// NetworkConnect
// ---------------------------------------------------------------------------

/// Output of connecting a container to a network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConnectOutput {
    /// The network name.
    pub network: String,
    /// The container ID or name.
    pub container: String,
}

/// Connect a container to a network.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::networks::NetworkConnect;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = NetworkConnect::new(&client, "my-network", "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct NetworkConnect {
    docker: Docker,
    network: String,
    container: String,
}

impl NetworkConnect {
    /// Create a new network-connect operation.
    pub fn new(
        client: impl Into<DockerRef>,
        network: impl Into<String>,
        container: impl Into<String>,
    ) -> Self {
        Self {
            docker: client.into().0,
            network: network.into(),
            container: container.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the network or container does
    /// not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<NetworkConnectOutput, OperationError> {
        let request = NetworkConnectRequest {
            container: self.container.clone(),
            ..Default::default()
        };
        self.docker
            .connect_network(&self.network, request)
            .await
            .map_err(docker_error)?;
        Ok(NetworkConnectOutput {
            network: self.network.clone(),
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for NetworkConnect {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "network_connect",
            "network": self.network,
            "container": self.container,
        }))
    }
}

impl TypedOperation for NetworkConnect {
    type Output = NetworkConnectOutput;
}

// ---------------------------------------------------------------------------
// NetworkDisconnect
// ---------------------------------------------------------------------------

/// Output of disconnecting a container from a network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkDisconnectOutput {
    /// The network name.
    pub network: String,
    /// The container ID or name.
    pub container: String,
}

/// Disconnect a container from a network.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::networks::NetworkDisconnect;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = NetworkDisconnect::new(&client, "my-network", "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct NetworkDisconnect {
    docker: Docker,
    network: String,
    container: String,
}

impl NetworkDisconnect {
    /// Create a new network-disconnect operation.
    pub fn new(
        client: impl Into<DockerRef>,
        network: impl Into<String>,
        container: impl Into<String>,
    ) -> Self {
        Self {
            docker: client.into().0,
            network: network.into(),
            container: container.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the network or container does
    /// not exist.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<NetworkDisconnectOutput, OperationError> {
        let request = NetworkDisconnectRequest {
            container: self.container.clone(),
            force: Some(false),
        };
        self.docker
            .disconnect_network(&self.network, request)
            .await
            .map_err(docker_error)?;
        Ok(NetworkDisconnectOutput {
            network: self.network.clone(),
            container: self.container.clone(),
        })
    }
}

#[async_trait]
impl Operation for NetworkDisconnect {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "network_disconnect",
            "network": self.network,
            "container": self.container,
        }))
    }
}

impl TypedOperation for NetworkDisconnect {
    type Output = NetworkDisconnectOutput;
}
