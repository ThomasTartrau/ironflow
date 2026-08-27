//! Local Claude Code CLI provider.
//!
//! [`ClaudeCodeProvider`] spawns the `claude` binary as a local child process.
//! This is the default transport and requires the `claude` CLI to be installed
//! on the same machine.
//!
//! # Requirements
//!
//! The `claude` binary must be available on `$PATH`. Install it via
//! `npm install -g @anthropic-ai/claude-code`.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let provider = ClaudeCodeProvider::new();
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

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time;
use tracing::{debug, warn};

use crate::error::AgentError;
use crate::provider::{AgentConfig, AgentInput, AgentProvider, InvokeFuture};
use crate::utils::truncate_output;

use super::common::{self, DEFAULT_TIMEOUT};

/// Download every declared input to its `mount_path` on the local filesystem.
///
/// Creates intermediate directories as needed. Returns an error if any download
/// fails or the path cannot be written. Idempotent: existing files are
/// overwritten.
async fn materialize_inputs_local(inputs: &[AgentInput]) -> Result<(), AgentError> {
    if inputs.is_empty() {
        return Ok(());
    }

    let client = reqwest::Client::new();
    for input in inputs {
        if !input.mount_path.starts_with('/') {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!(
                    "agent input mount_path must be absolute, got '{}'",
                    input.mount_path
                ),
            });
        }
        if let Some(parent) = Path::new(&input.mount_path).parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!(
                        "failed to create input parent dir '{}': {e}",
                        parent.display()
                    ),
                })?;
        }
        let resp = client
            .get(&input.url)
            .send()
            .await
            .map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to fetch input '{}': {e}", input.url),
            })?;
        if !resp.status().is_success() {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!(
                    "input fetch '{}' returned HTTP {}",
                    input.url,
                    resp.status()
                ),
            });
        }
        let bytes = resp.bytes().await.map_err(|e| AgentError::ProcessFailed {
            exit_code: -1,
            stderr: format!("failed to read input body '{}': {e}", input.url),
        })?;
        fs::write(&input.mount_path, &bytes)
            .await
            .map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to write input '{}': {e}", input.mount_path),
            })?;
        debug!(
            url = %input.url,
            path = %input.mount_path,
            bytes = bytes.len(),
            "materialized agent input"
        );
    }
    Ok(())
}

/// [`AgentProvider`] that shells out to the
/// `claude` CLI on the local machine.
///
/// The provider spawns a `claude` child process for each invocation, passing
/// the prompt and configuration as command-line arguments. The `CLAUDECODE`
/// environment variable is removed to avoid recursive invocation when running
/// inside Claude Code itself.
#[derive(Clone)]
pub struct ClaudeCodeProvider {
    /// Maximum wall-clock time to wait for the `claude` process.
    pub(crate) timeout: Duration,
}

impl ClaudeCodeProvider {
    /// Create a new provider with the default timeout of 5 minutes.
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Override the default timeout.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    ///
    /// let provider = ClaudeCodeProvider::new()
    ///     .timeout(Duration::from_secs(600));
    /// ```
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Default for ClaudeCodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentProvider for ClaudeCodeProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            common::validate_prompt_size(config)?;
            materialize_inputs_local(&config.inputs).await?;
            let built = common::build_command(config)?;

            debug!(
                model = %config.model,
                has_system_prompt = config.system_prompt.is_some(),
                has_json_schema = config.json_schema.is_some(),
                has_tools = !config.allowed_tools.is_empty(),
                tools = ?config.allowed_tools,
                permission_mode = ?config.permission_mode,
                verbose = config.verbose,
                arg_count = built.args.len(),
                prompt_via_stdin = built.stdin_prompt.is_some(),
                "spawning claude process"
            );

            let start = Instant::now();

            let mut cmd = Command::new("claude");
            cmd.args(&built.args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);

            if built.stdin_prompt.is_some() {
                cmd.stdin(Stdio::piped());
            }

            for var in common::env_vars_to_remove() {
                cmd.env_remove(&var);
            }

            if let Some(ref ctx) = config.trace_context {
                cmd.env("TRACEPARENT", ctx.to_traceparent());
            }

            if let Some(ref dir) = config.working_dir {
                cmd.current_dir(dir);
            }

            let mut child = cmd.spawn().map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to spawn claude: {e}"),
            })?;

            if let Some(ref prompt) = built.stdin_prompt {
                let mut stdin = child.stdin.take().expect("stdin was piped");
                let prompt_bytes = prompt.as_bytes().to_vec();
                let write_fut = async move {
                    stdin.write_all(&prompt_bytes).await?;
                    stdin.shutdown().await?;
                    Ok::<_, std::io::Error>(())
                };
                let wait_fut = child.wait_with_output();
                let (write_result, wait_result) =
                    match time::timeout(self.timeout, async { tokio::join!(write_fut, wait_fut) })
                        .await
                    {
                        Ok(pair) => pair,
                        Err(_) => {
                            warn!(timeout = ?self.timeout, "claude process timed out");
                            return Err(AgentError::Timeout {
                                limit: self.timeout,
                            });
                        }
                    };

                write_result.map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to write prompt to stdin: {e}"),
                })?;

                let output = wait_result.map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to wait for claude: {e}"),
                })?;

                let duration_ms = start.elapsed().as_millis() as u64;
                let stdout = truncate_output(&output.stdout, "claude stdout");

                if !output.status.success() {
                    let exit_code = output.status.code().unwrap_or(-1);
                    let stderr = truncate_output(&output.stderr, "claude stderr");
                    return common::handle_nonzero_exit(
                        exit_code,
                        &stdout,
                        &stderr,
                        config,
                        duration_ms,
                        "local",
                    );
                }

                debug!(stdout_len = stdout.len(), "claude process completed");
                return common::parse_output(&stdout, config, duration_ms);
            }

            let output = match time::timeout(self.timeout, child.wait_with_output()).await {
                Ok(result) => result.map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to wait for claude: {e}"),
                })?,
                Err(_) => {
                    warn!(timeout = ?self.timeout, "claude process timed out");
                    return Err(AgentError::Timeout {
                        limit: self.timeout,
                    });
                }
            };

            let duration_ms = start.elapsed().as_millis() as u64;

            let stdout = truncate_output(&output.stdout, "claude stdout");

            if !output.status.success() {
                let exit_code = output.status.code().unwrap_or(-1);
                let stderr = truncate_output(&output.stderr, "claude stderr");
                return common::handle_nonzero_exit(
                    exit_code,
                    &stdout,
                    &stderr,
                    config,
                    duration_ms,
                    "local",
                );
            }

            debug!(stdout_len = stdout.len(), "claude process completed");

            common::parse_output(&stdout, config, duration_ms)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_default_timeout() {
        let provider = ClaudeCodeProvider::new();
        assert_eq!(provider.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn provider_custom_timeout() {
        let provider = ClaudeCodeProvider::new().timeout(Duration::from_secs(600));
        assert_eq!(provider.timeout, Duration::from_secs(600));
    }

    #[test]
    fn provider_default_matches_new() {
        let from_new = ClaudeCodeProvider::new();
        let from_default = ClaudeCodeProvider::default();
        assert_eq!(from_new.timeout, from_default.timeout);
    }

    #[test]
    fn provider_clone() {
        let provider = ClaudeCodeProvider::new().timeout(Duration::from_secs(42));
        let cloned = provider.clone();
        assert_eq!(cloned.timeout, Duration::from_secs(42));
    }
}
