//! Docker transport for Claude Code CLI.
//!
//! [`DockerProvider`] executes the `claude` CLI inside a running Docker
//! container via the Docker Engine API (`docker exec`). This is useful when
//! the Claude CLI and its credentials are isolated in a container.
//!
//! # Requirements
//!
//! * A Docker daemon must be reachable (local socket or remote TCP).
//! * The target container must be running and have the `claude` binary installed.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//! use ironflow_core::providers::claude::DockerProvider;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let provider = DockerProvider::new("claude-worker");
//!
//! let result = Agent::new()
//!     .prompt("What is 2 + 2?")
//!     .run(&provider)
//!     .await?;
//!
//! println!("{}", result.text());
//! # Ok(())
//! # }
//! ```

use std::time::{Duration, Instant};

use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecResults};
use futures_util::StreamExt;
use tracing::{debug, error, warn};

use crate::error::AgentError;
use crate::provider::{AgentConfig, AgentProvider, InvokeFuture};

use super::common::{self, DEFAULT_TIMEOUT};

/// [`AgentProvider`] that executes the `claude` CLI inside a Docker container.
///
/// Uses the Docker Engine API (via [`bollard`]) to create an exec instance,
/// start it, and capture stdout/stderr. No PTY is allocated and stdin is
/// not attached, so the execution is fully non-interactive.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::claude::DockerProvider;
///
/// let provider = DockerProvider::new("claude-container")
///     .user("node")
///     .working_dir("/workspace");
/// ```
#[derive(Clone)]
pub struct DockerProvider {
    container: String,
    docker_host: Option<String>,
    user: Option<String>,
    claude_path: String,
    working_dir: Option<String>,
    timeout: Duration,
}

impl DockerProvider {
    /// Create a new Docker provider targeting the given container name or ID.
    pub fn new(container: &str) -> Self {
        Self {
            container: container.to_string(),
            docker_host: None,
            user: None,
            claude_path: "claude".to_string(),
            working_dir: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Override the Docker daemon socket/URL (sets `DOCKER_HOST`).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::DockerProvider;
    ///
    /// let provider = DockerProvider::new("my-container")
    ///     .docker_host("tcp://192.168.1.100:2375");
    /// ```
    pub fn docker_host(mut self, host: &str) -> Self {
        self.docker_host = Some(host.to_string());
        self
    }

    /// Run the exec as a specific user inside the container.
    pub fn user(mut self, user: &str) -> Self {
        self.user = Some(user.to_string());
        self
    }

    /// Override the path to the `claude` binary inside the container (default: `"claude"`).
    pub fn claude_path(mut self, path: &str) -> Self {
        self.claude_path = path.to_string();
        self
    }

    /// Set the working directory inside the container.
    pub fn working_dir(mut self, dir: &str) -> Self {
        self.working_dir = Some(dir.to_string());
        self
    }

    /// Override the default timeout (default: 5 minutes).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Connect to the Docker daemon.
    fn connect(&self) -> Result<Docker, AgentError> {
        let docker = match &self.docker_host {
            Some(host) => Docker::connect_with_http(host, 120, bollard::API_DEFAULT_VERSION),
            None => Docker::connect_with_socket_defaults(),
        };
        docker.map_err(|e| AgentError::ProcessFailed {
            exit_code: -1,
            stderr: format!("failed to connect to Docker daemon: {e}"),
        })
    }
}

impl AgentProvider for DockerProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            let args = common::build_args(config)?;

            let mut cmd: Vec<String> = vec![self.claude_path.clone()];
            cmd.extend(args);

            let work_dir = config
                .working_dir
                .as_deref()
                .or(self.working_dir.as_deref());

            debug!(
                container = %self.container,
                model = %config.model,
                working_dir = ?work_dir,
                "creating docker exec"
            );

            let start = Instant::now();

            let docker = self.connect()?;

            // Clear all CLAUDE* and IRONFLOW_ALLOW_BYPASS env vars to prevent
            // sub-agent mode interference inside the container.
            let env_clear: Vec<String> = common::env_vars_to_remove()
                .iter()
                .map(|var| format!("{var}="))
                .collect();

            // Create exec instance
            let exec_config = CreateExecOptions {
                cmd: Some(cmd.iter().map(|s| s.as_str()).collect()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                attach_stdin: Some(false),
                tty: Some(false),
                working_dir: work_dir,
                user: self.user.as_deref(),
                env: Some(env_clear.iter().map(|s| s.as_str()).collect()),
                ..Default::default()
            };

            let exec = docker
                .create_exec(&self.container, exec_config)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to create docker exec: {e}"),
                })?;

            // Start exec and collect output
            let start_result = docker
                .start_exec(&exec.id, None::<bollard::exec::StartExecOptions>)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to start docker exec: {e}"),
                })?;

            let mut stdout_buf = Vec::new();
            let mut stderr_buf = Vec::new();

            let collect_result = tokio::time::timeout(self.timeout, async {
                match start_result {
                    StartExecResults::Attached { mut output, .. } => {
                        while let Some(result) = output.next().await {
                            match result {
                                Ok(bollard::container::LogOutput::StdOut { message }) => {
                                    stdout_buf.extend_from_slice(&message);
                                }
                                Ok(bollard::container::LogOutput::StdErr { message }) => {
                                    stderr_buf.extend_from_slice(&message);
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    warn!("docker exec stream error: {e}");
                                    break;
                                }
                            }
                        }
                    }
                    StartExecResults::Detached => {
                        return Err(AgentError::ProcessFailed {
                            exit_code: -1,
                            stderr: "docker exec returned Detached mode, cannot capture output"
                                .to_string(),
                        });
                    }
                }
                Ok(())
            })
            .await;

            match collect_result {
                Err(_) => {
                    warn!(timeout = ?self.timeout, "docker exec timed out");
                    return Err(AgentError::Timeout {
                        limit: self.timeout,
                    });
                }
                Ok(Err(e)) => return Err(e),
                Ok(Ok(())) => {}
            }

            let duration_ms = start.elapsed().as_millis() as u64;

            // Inspect exec to get exit code
            let inspect =
                docker
                    .inspect_exec(&exec.id)
                    .await
                    .map_err(|e| AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!("failed to inspect docker exec: {e}"),
                    })?;

            let exit_code = inspect.exit_code.unwrap_or(1) as i32;

            let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
            let stderr = String::from_utf8_lossy(&stderr_buf).to_string();

            if exit_code != 0 {
                let error_detail = if stderr.is_empty() {
                    if stdout.is_empty() {
                        "(no output captured)".to_string()
                    } else {
                        stdout.clone()
                    }
                } else {
                    stderr
                };

                error!(
                    exit_code,
                    error_detail_len = error_detail.len(),
                    "docker claude process failed"
                );
                return Err(AgentError::ProcessFailed {
                    exit_code,
                    stderr: error_detail,
                });
            }

            debug!(stdout_len = stdout.len(), "docker claude process completed");

            common::parse_response(&stdout, config, duration_ms)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_provider_defaults() {
        let provider = DockerProvider::new("my-container");
        assert_eq!(provider.container, "my-container");
        assert!(provider.docker_host.is_none());
        assert!(provider.user.is_none());
        assert_eq!(provider.claude_path, "claude");
        assert!(provider.working_dir.is_none());
        assert_eq!(provider.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn docker_provider_builder_chain() {
        let provider = DockerProvider::new("cnt")
            .docker_host("tcp://remote:2375")
            .user("node")
            .claude_path("/usr/bin/claude")
            .working_dir("/workspace")
            .timeout(Duration::from_secs(600));

        assert_eq!(provider.container, "cnt");
        assert_eq!(provider.docker_host, Some("tcp://remote:2375".to_string()));
        assert_eq!(provider.user, Some("node".to_string()));
        assert_eq!(provider.claude_path, "/usr/bin/claude");
        assert_eq!(provider.working_dir, Some("/workspace".to_string()));
        assert_eq!(provider.timeout, Duration::from_secs(600));
    }

    #[test]
    fn docker_provider_clone() {
        let provider = DockerProvider::new("cnt")
            .user("root")
            .timeout(Duration::from_secs(42));
        let cloned = provider.clone();
        assert_eq!(cloned.container, "cnt");
        assert_eq!(cloned.user, Some("root".to_string()));
        assert_eq!(cloned.timeout, Duration::from_secs(42));
    }
}
