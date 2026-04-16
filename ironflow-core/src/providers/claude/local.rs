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

use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::time;
use tracing::{debug, warn};

use crate::error::AgentError;
use crate::provider::{AgentConfig, AgentProvider, InvokeFuture};
use crate::utils::truncate_output;

use super::common::{self, DEFAULT_TIMEOUT};

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
            let args = common::build_args(config)?;

            debug!(
                model = %config.model,
                has_system_prompt = config.system_prompt.is_some(),
                has_json_schema = config.json_schema.is_some(),
                has_tools = !config.allowed_tools.is_empty(),
                tools = ?config.allowed_tools,
                permission_mode = ?config.permission_mode,
                verbose = config.verbose,
                arg_count = args.len(),
                "spawning claude process"
            );

            let start = Instant::now();

            let mut cmd = Command::new("claude");
            cmd.args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);

            // Remove ALL inherited Claude Code env vars to prevent
            // sub-agent mode interference (model override, entrypoint detection, etc.)
            for var in common::env_vars_to_remove() {
                cmd.env_remove(&var);
            }

            if let Some(ref dir) = config.working_dir {
                cmd.current_dir(dir);
            }

            let child = cmd.spawn().map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to spawn claude: {e}"),
            })?;

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
