//! Container I/O operations: logs, exec, wait.

use async_trait::async_trait;
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::query_parameters::{LogsOptions, WaitContainerOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_stream::StreamExt;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// ContainerLogs
// ---------------------------------------------------------------------------

/// Output of a container logs retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerLogsOutput {
    /// The log lines.
    pub lines: Vec<String>,
}

/// Retrieve logs from a container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerLogs;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerLogs::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerLogs {
    docker: Docker,
    container: String,
    stdout: bool,
    stderr: bool,
    tail: Option<String>,
}

impl ContainerLogs {
    /// Create a new container-logs operation.
    pub fn new(client: impl Into<DockerRef>, container: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
            stdout: true,
            stderr: true,
            tail: None,
        }
    }

    /// Only retrieve stdout.
    pub fn stdout_only(mut self) -> Self {
        self.stderr = false;
        self
    }

    /// Only retrieve stderr.
    pub fn stderr_only(mut self) -> Self {
        self.stdout = false;
        self
    }

    /// Limit the number of lines from the end.
    pub fn tail(mut self, n: u64) -> Self {
        self.tail = Some(n.to_string());
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
    ) -> Result<ContainerLogsOutput, OperationError> {
        let options = LogsOptions {
            stdout: self.stdout,
            stderr: self.stderr,
            tail: self.tail.clone().unwrap_or_else(|| "all".to_string()),
            ..Default::default()
        };
        let mut stream = self.docker.logs(&self.container, Some(options));
        let mut lines = Vec::new();
        while let Some(result) = stream.next().await {
            let chunk = result.map_err(docker_error)?;
            lines.push(chunk.to_string());
        }
        Ok(ContainerLogsOutput { lines })
    }
}

#[async_trait]
impl Operation for ContainerLogs {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_logs",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerLogs {
    type Output = ContainerLogsOutput;
}

// ---------------------------------------------------------------------------
// ContainerExec
// ---------------------------------------------------------------------------

/// Output of a container exec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerExecOutput {
    /// Combined stdout and stderr output.
    pub output: Vec<String>,
}

/// Execute a command inside a running container.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerExec;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerExec::new(&client, "my-container", vec!["echo", "hello"]);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerExec {
    docker: Docker,
    container: String,
    cmd: Vec<String>,
}

impl ContainerExec {
    /// Create a new container-exec operation.
    pub fn new(
        client: impl Into<DockerRef>,
        container: impl Into<String>,
        cmd: Vec<impl Into<String>>,
    ) -> Self {
        Self {
            docker: client.into().0,
            container: container.into(),
            cmd: cmd.into_iter().map(Into::into).collect(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the container does not exist or
    /// the command fails.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ContainerExecOutput, OperationError> {
        let exec_options = CreateExecOptions {
            cmd: Some(self.cmd.clone()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };
        let exec = self
            .docker
            .create_exec(&self.container, exec_options)
            .await
            .map_err(docker_error)?;
        let start_result = self
            .docker
            .start_exec(&exec.id, None)
            .await
            .map_err(docker_error)?;
        let mut output = Vec::new();
        if let StartExecResults::Attached {
            output: mut stream, ..
        } = start_result
        {
            while let Some(result) = stream.next().await {
                let chunk = result.map_err(docker_error)?;
                output.push(chunk.to_string());
            }
        }
        Ok(ContainerExecOutput { output })
    }
}

#[async_trait]
impl Operation for ContainerExec {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_exec",
            "container": self.container,
            "cmd": self.cmd,
        }))
    }
}

impl TypedOperation for ContainerExec {
    type Output = ContainerExecOutput;
}

// ---------------------------------------------------------------------------
// ContainerWait
// ---------------------------------------------------------------------------

/// Output of waiting for a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerWaitOutput {
    /// The exit code of the container.
    pub status_code: i64,
}

/// Wait for a container to stop and return its exit code.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::containers::ContainerWait;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ContainerWait::new(&client, "my-container");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ContainerWait {
    docker: Docker,
    container: String,
}

impl ContainerWait {
    /// Create a new container-wait operation.
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
    ) -> Result<ContainerWaitOutput, OperationError> {
        let options = WaitContainerOptions {
            condition: "not-running".to_string(),
        };
        let mut stream = self.docker.wait_container(&self.container, Some(options));
        let mut status_code = 0i64;
        while let Some(result) = stream.next().await {
            let response = result.map_err(docker_error)?;
            status_code = response.status_code;
        }
        Ok(ContainerWaitOutput { status_code })
    }
}

#[async_trait]
impl Operation for ContainerWait {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "container_wait",
            "container": self.container,
        }))
    }
}

impl TypedOperation for ContainerWait {
    type Output = ContainerWaitOutput;
}
