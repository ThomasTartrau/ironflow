//! Tool for executing shell commands.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::timeout;

use super::tool_trait::{Tool, ToolError, ToolOutput};

/// Default command timeout (60 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Maximum output size (1 MB).
const MAX_OUTPUT_SIZE: usize = 1024 * 1024;

/// Executes shell commands and returns stdout/stderr.
///
/// # Security
///
/// Commands run with the same permissions as the ironflow process.
/// Use `with_timeout` to prevent runaway processes. An optional
/// `working_dir` constrains where commands execute.
pub struct BashTool {
    timeout: Duration,
    working_dir: Option<String>,
}

impl BashTool {
    /// Create a `BashTool` with default timeout (60s).
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            working_dir: None,
        }
    }

    /// Set the command timeout.
    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.timeout = duration;
        self
    }

    /// Set the working directory for all commands.
    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_SIZE {
        output.to_string()
    } else {
        let truncated = &output[..output.floor_char_boundary(MAX_OUTPUT_SIZE)];
        format!("{truncated}\n... (output truncated at {MAX_OUTPUT_SIZE} bytes)")
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its stdout and stderr. Use for running programs, scripts, and system commands."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 60)"
                }
            },
            "required": ["command"]
        })
    }

    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let command_str = input
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::new("missing 'command' parameter"))?;

            let cmd_timeout = input
                .get("timeout_secs")
                .and_then(|v| v.as_u64())
                .map(Duration::from_secs)
                .unwrap_or(self.timeout);

            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(command_str);
            cmd.kill_on_drop(true);

            if let Some(ref dir) = self.working_dir {
                cmd.current_dir(dir);
            }

            let output = match timeout(cmd_timeout, cmd.output()).await {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    return Ok(ToolOutput::error(format!("Failed to execute command: {e}")));
                }
                Err(_) => {
                    return Ok(ToolOutput::error(format!(
                        "Command timed out after {}s: {command_str}",
                        cmd_timeout.as_secs()
                    )));
                }
            };

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&truncate_output(&stdout));
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str("[stderr]\n");
                result.push_str(&truncate_output(&stderr));
            }

            if result.is_empty() {
                result = format!("(exit code: {exit_code})");
            } else if exit_code != 0 {
                result.push_str(&format!("\n(exit code: {exit_code})"));
            }

            if exit_code == 0 {
                Ok(ToolOutput::success(result))
            } else {
                Ok(ToolOutput::error(result))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn bash_echo() {
        let tool = BashTool::new();
        let result = tool
            .execute(json!({"command": "echo hello world"}))
            .await
            .expect("should succeed");
        assert!(!result.is_error);
        assert!(result.content.contains("hello world"));
    }

    #[tokio::test]
    async fn bash_exit_code_nonzero() {
        let tool = BashTool::new();
        let result = tool
            .execute(json!({"command": "exit 42"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("exit code: 42"));
    }

    #[tokio::test]
    async fn bash_stderr_captured() {
        let tool = BashTool::new();
        let result = tool
            .execute(json!({"command": "echo err >&2 && exit 1"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("[stderr]"));
        assert!(result.content.contains("err"));
    }

    #[tokio::test]
    async fn bash_timeout() {
        let tool = BashTool::new().with_timeout(Duration::from_millis(100));
        let result = tool
            .execute(json!({"command": "sleep 10"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("timed out"));
    }

    #[tokio::test]
    async fn bash_custom_timeout_param() {
        let tool = BashTool::new();
        let result = tool
            .execute(json!({"command": "sleep 10", "timeout_secs": 1}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("timed out"));
    }

    #[tokio::test]
    async fn bash_working_dir() {
        let tool = BashTool::new().with_working_dir("/tmp");
        let result = tool
            .execute(json!({"command": "pwd"}))
            .await
            .expect("should succeed");
        assert!(!result.is_error);
        // macOS uses /private/tmp
        assert!(result.content.contains("/tmp") || result.content.contains("/private/tmp"));
    }

    #[tokio::test]
    async fn bash_missing_command_param() {
        let tool = BashTool::new();
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }
}
