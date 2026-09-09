//! System operations: info, version, ping, df, prune.

use async_trait::async_trait;
use bollard::Docker;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// SystemInfo
// ---------------------------------------------------------------------------

/// Output of system info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfoOutput {
    /// The full system info as JSON.
    pub data: Value,
}

/// Get system-wide information from the Docker daemon.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::system::SystemInfo;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = SystemInfo::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct SystemInfo {
    docker: Docker,
}

impl SystemInfo {
    /// Create a new system-info operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the Docker daemon is
    /// unreachable.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<SystemInfoOutput, OperationError> {
        let info = self.docker.info().await.map_err(docker_error)?;
        let data = serde_json::to_value(&info).map_err(|e| OperationError::External {
            origin: "docker".to_string(),
            message: e.to_string(),
        })?;
        Ok(SystemInfoOutput { data })
    }
}

#[async_trait]
impl Operation for SystemInfo {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "operation": "system_info" }))
    }
}

impl TypedOperation for SystemInfo {
    type Output = SystemInfoOutput;
}

// ---------------------------------------------------------------------------
// SystemVersion
// ---------------------------------------------------------------------------

/// Output of system version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemVersionOutput {
    /// The full version info as JSON.
    pub data: Value,
}

/// Get version information from the Docker daemon.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::system::SystemVersion;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = SystemVersion::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct SystemVersion {
    docker: Docker,
}

impl SystemVersion {
    /// Create a new system-version operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
        }
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
    ) -> Result<SystemVersionOutput, OperationError> {
        let version = self.docker.version().await.map_err(docker_error)?;
        let data = serde_json::to_value(&version).map_err(|e| OperationError::External {
            origin: "docker".to_string(),
            message: e.to_string(),
        })?;
        Ok(SystemVersionOutput { data })
    }
}

#[async_trait]
impl Operation for SystemVersion {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "operation": "system_version" }))
    }
}

impl TypedOperation for SystemVersion {
    type Output = SystemVersionOutput;
}

// ---------------------------------------------------------------------------
// SystemPing
// ---------------------------------------------------------------------------

/// Output of a system ping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemPingOutput {
    /// The ping response (typically "OK").
    pub response: String,
}

/// Ping the Docker daemon.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::system::SystemPing;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = SystemPing::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct SystemPing {
    docker: Docker,
}

impl SystemPing {
    /// Create a new system-ping operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the Docker daemon is
    /// unreachable.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<SystemPingOutput, OperationError> {
        let response = self.docker.ping().await.map_err(docker_error)?;
        Ok(SystemPingOutput { response })
    }
}

#[async_trait]
impl Operation for SystemPing {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "operation": "system_ping" }))
    }
}

impl TypedOperation for SystemPing {
    type Output = SystemPingOutput;
}

// ---------------------------------------------------------------------------
// SystemDf
// ---------------------------------------------------------------------------

/// Output of system disk usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemDfOutput {
    /// The full disk usage data as JSON.
    pub data: Value,
}

/// Get disk usage information from the Docker daemon.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::system::SystemDf;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = SystemDf::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct SystemDf {
    docker: Docker,
}

impl SystemDf {
    /// Create a new system-df operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the Docker daemon is
    /// unreachable.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<SystemDfOutput, OperationError> {
        let df = self
            .docker
            .df(None::<bollard::query_parameters::DataUsageOptions>)
            .await
            .map_err(docker_error)?;
        let data = serde_json::to_value(&df).map_err(|e| OperationError::External {
            origin: "docker".to_string(),
            message: e.to_string(),
        })?;
        Ok(SystemDfOutput { data })
    }
}

#[async_trait]
impl Operation for SystemDf {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "operation": "system_df" }))
    }
}

impl TypedOperation for SystemDf {
    type Output = SystemDfOutput;
}
