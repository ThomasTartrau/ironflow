//! Shell step executor.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use serde_json::json;
use tracing::info;

use ironflow_core::operations::shell::Shell;
use ironflow_core::provider::AgentProvider;

use crate::config::ShellConfig;
use crate::error::EngineError;

use super::{StepExecutor, StepOutput};

/// Executor for shell steps.
///
/// Runs a shell command and captures stdout, stderr, and exit code.
pub struct ShellExecutor<'a> {
    config: &'a ShellConfig,
}

impl<'a> ShellExecutor<'a> {
    /// Create a new shell executor from a config reference.
    pub fn new(config: &'a ShellConfig) -> Self {
        Self { config }
    }
}

impl StepExecutor for ShellExecutor<'_> {
    async fn execute(&self, _provider: &Arc<dyn AgentProvider>) -> Result<StepOutput, EngineError> {
        let start = Instant::now();

        let mut shell = Shell::new(&self.config.command);
        if let Some(secs) = self.config.timeout_secs {
            shell = shell.timeout(Duration::from_secs(secs));
        }
        if let Some(ref dir) = self.config.dir {
            shell = shell.dir(dir);
        }
        for (key, value) in &self.config.env {
            shell = shell.env(key, value);
        }
        if self.config.clean_env {
            shell = shell.clean_env();
        }

        let output = shell.run().await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            step_kind = "shell",
            command = %self.config.command,
            exit_code = output.exit_code(),
            duration_ms,
            "shell step completed"
        );

        Ok(StepOutput {
            output: json!({
                "stdout": output.stdout(),
                "stderr": output.stderr(),
                "exit_code": output.exit_code(),
            }),
            duration_ms,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_core::providers::record_replay::RecordReplayProvider;

    fn create_test_provider() -> Arc<dyn AgentProvider> {
        let inner = ClaudeCodeProvider::new();
        Arc::new(RecordReplayProvider::replay(
            inner,
            "/tmp/ironflow-fixtures",
        ))
    }

    #[tokio::test]
    async fn shell_simple_command() {
        let config = ShellConfig::new("echo hello");
        let executor = ShellExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.output["exit_code"].as_i64().unwrap(), 0);
        assert!(output.output["stdout"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn shell_nonzero_exit_returns_error() {
        let config = ShellConfig::new("exit 1");
        let executor = ShellExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shell_env_variables() {
        let config = ShellConfig::new("echo $MY_VAR").env("MY_VAR", "test_value");
        let executor = ShellExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(
            output.output["stdout"]
                .as_str()
                .unwrap()
                .contains("test_value")
        );
    }

    #[tokio::test]
    async fn shell_step_output_has_structure() {
        let config = ShellConfig::new("echo test");
        let executor = ShellExecutor::new(&config);
        let provider = create_test_provider();

        let output = executor.execute(&provider).await.unwrap();
        assert!(output.output.get("stdout").is_some());
        assert!(output.output.get("stderr").is_some());
        assert!(output.output.get("exit_code").is_some());
        assert_eq!(output.cost_usd, Decimal::ZERO);
        // duration_ms can be 0 when the command completes in under 1ms (CI)
        assert!(output.duration_ms < 5000);
    }

    #[tokio::test]
    async fn shell_command_with_pipe() {
        let config = ShellConfig::new("echo hello | grep hello");
        let executor = ShellExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.output["exit_code"].as_i64().unwrap(), 0);
        assert!(output.output["stdout"].as_str().unwrap().contains("hello"));
    }
}
