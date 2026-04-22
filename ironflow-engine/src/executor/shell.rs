//! Shell step executor.

use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::spawn;
use tracing::info;

use ironflow_core::error::OperationError;
use ironflow_core::operations::shell::Shell;
use ironflow_core::provider::AgentProvider;
use ironflow_core::utils::truncate_output;

use crate::config::ShellConfig;
use crate::error::EngineError;
use crate::log_sender::StepLogSender;
use crate::notify::LogStream;

use super::{StepExecutor, StepOutput};

const DEFAULT_SHELL_TIMEOUT: Duration = Duration::from_secs(300);

/// Read lines from an async reader, emit each line to the sender, and
/// accumulate the full output as a single `String`.
async fn read_and_stream<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    sender: StepLogSender,
    stream: LogStream,
) -> String {
    let mut lines = BufReader::new(reader).lines();
    let mut collected = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        sender.emit(stream, &line);
        if !collected.is_empty() {
            collected.push('\n');
        }
        collected.push_str(&line);
    }
    collected
}

/// Executor for shell steps.
///
/// Runs a shell command and captures stdout, stderr, and exit code.
/// When a [`StepLogSender`] is attached, stdout and stderr are streamed
/// line-by-line in real time.
pub struct ShellExecutor<'a> {
    config: &'a ShellConfig,
    log_sender: Option<StepLogSender>,
}

impl<'a> ShellExecutor<'a> {
    /// Create a new shell executor from a config reference.
    pub fn new(config: &'a ShellConfig) -> Self {
        Self {
            config,
            log_sender: None,
        }
    }

    /// Attach a log sender for real-time line streaming.
    pub fn with_log_sender(mut self, sender: StepLogSender) -> Self {
        self.log_sender = Some(sender);
        self
    }
}

impl StepExecutor for ShellExecutor<'_> {
    async fn execute(&self, _provider: &Arc<dyn AgentProvider>) -> Result<StepOutput, EngineError> {
        match self.log_sender {
            Some(ref sender) => self.execute_streaming(sender.clone()).await,
            None => self.execute_buffered().await,
        }
    }
}

impl ShellExecutor<'_> {
    /// Non-streaming execution via [`Shell::run()`].
    async fn execute_buffered(&self) -> Result<StepOutput, EngineError> {
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

        self.record_metrics(duration_ms);

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
            debug_messages: None,
        })
    }

    /// Streaming execution: reads stdout/stderr line-by-line and forwards
    /// each line to the [`StepLogSender`] in real time.
    async fn execute_streaming(&self, sender: StepLogSender) -> Result<StepOutput, EngineError> {
        let start = Instant::now();

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(&self.config.command);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if self.config.clean_env {
            cmd.env_clear();
        }
        if let Some(ref dir) = self.config.dir {
            cmd.current_dir(dir);
        }
        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            EngineError::Operation(OperationError::Shell {
                exit_code: -1,
                stderr: format!("failed to spawn shell: {e}"),
            })
        })?;

        let stdout_pipe = child.stdout.take().expect("stdout piped");
        let stderr_pipe = child.stderr.take().expect("stderr piped");

        let stdout_task = spawn(read_and_stream(
            stdout_pipe,
            sender.clone(),
            LogStream::Stdout,
        ));
        let stderr_task = spawn(read_and_stream(stderr_pipe, sender, LogStream::Stderr));

        let timeout_dur = self
            .config
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_SHELL_TIMEOUT);

        let status = match tokio::time::timeout(timeout_dur, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                return Err(EngineError::Operation(OperationError::Shell {
                    exit_code: -1,
                    stderr: format!("failed to wait for shell: {e}"),
                }));
            }
            Err(_) => {
                child.kill().await.ok();
                return Err(EngineError::Operation(OperationError::Timeout {
                    step: self.config.command.clone(),
                    limit: timeout_dur,
                }));
            }
        };

        let raw_stdout = stdout_task.await.unwrap_or_default();
        let raw_stderr = stderr_task.await.unwrap_or_default();

        let stdout = truncate_output(raw_stdout.as_bytes(), "shell stdout");
        let stderr = truncate_output(raw_stderr.as_bytes(), "shell stderr");

        let exit_code = status.code().unwrap_or(-1);
        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            step_kind = "shell",
            command = %self.config.command,
            exit_code,
            duration_ms,
            streaming = true,
            "shell step completed"
        );

        self.record_metrics(duration_ms);

        if exit_code != 0 {
            return Err(EngineError::Operation(OperationError::Shell {
                exit_code,
                stderr: stderr.clone(),
            }));
        }

        Ok(StepOutput {
            output: json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": exit_code,
            }),
            duration_ms,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            debug_messages: None,
        })
    }

    #[allow(unused_variables)]
    fn record_metrics(&self, duration_ms: u64) {
        #[cfg(feature = "prometheus")]
        {
            use ironflow_core::metric_names::{
                SHELL_DURATION_SECONDS, SHELL_TOTAL, STATUS_SUCCESS,
            };
            use metrics::{counter, histogram};
            counter!(SHELL_TOTAL, "status" => STATUS_SUCCESS).increment(1);
            histogram!(SHELL_DURATION_SECONDS).record(duration_ms as f64 / 1000.0);
        }
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

    #[tokio::test]
    async fn shell_streaming_emits_lines() {
        let config = ShellConfig::new("echo line1 && echo line2");
        let (sender, mut receiver) = crate::log_sender::channel();
        let step_sender = StepLogSender::new(
            sender,
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            "test".to_string(),
        );
        let executor = ShellExecutor::new(&config).with_log_sender(step_sender);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.output["stdout"].as_str().unwrap().contains("line1"));
        assert!(output.output["stdout"].as_str().unwrap().contains("line2"));

        let mut lines = Vec::new();
        while let Ok(line) = receiver.try_recv() {
            lines.push(line);
        }
        assert!(lines.len() >= 2);
        assert_eq!(lines[0].stream, LogStream::Stdout);
        assert_eq!(lines[0].line, "line1");
        assert_eq!(lines[1].line, "line2");
    }

    #[tokio::test]
    async fn shell_streaming_captures_stderr() {
        let config = ShellConfig::new("echo err >&2");
        let (sender, mut receiver) = crate::log_sender::channel();
        let step_sender = StepLogSender::new(
            sender,
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            "test".to_string(),
        );
        let executor = ShellExecutor::new(&config).with_log_sender(step_sender);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());

        let mut stderr_lines = Vec::new();
        while let Ok(line) = receiver.try_recv() {
            if line.stream == LogStream::Stderr {
                stderr_lines.push(line);
            }
        }
        assert!(!stderr_lines.is_empty());
        assert_eq!(stderr_lines[0].line, "err");
    }

    #[tokio::test]
    async fn shell_streaming_nonzero_exit_returns_error() {
        let config = ShellConfig::new("exit 42");
        let (sender, _receiver) = crate::log_sender::channel();
        let step_sender = StepLogSender::new(
            sender,
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            "test".to_string(),
        );
        let executor = ShellExecutor::new(&config).with_log_sender(step_sender);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_err());
    }
}
