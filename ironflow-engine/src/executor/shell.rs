//! Shell step executor.

use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::spawn;
use tracing::info;

use ironflow_core::dry_run::is_dry_run;
use ironflow_core::error::OperationError;
use ironflow_core::operations::shell::Shell;
use ironflow_core::provider::AgentProvider;
use ironflow_core::utils::truncate_output;
use ironflow_store::entities::StepKind;

use crate::config::ShellConfig;
use crate::error::EngineError;
use crate::log_sender::StepLogSender;
use crate::notify::LogStream;

use super::{StepArtifacts, StepExecutor, StepOutput};

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

/// Exit code of a finished process: the real code, `-signal` when a signal
/// killed it, `-1` otherwise.
fn exit_code_of(status: ExitStatus) -> i32 {
    status
        .code()
        .or_else(|| status.signal().map(|signal| -signal))
        .unwrap_or(-1)
}

/// Build the output JSON shared by the buffered and streaming paths.
fn build_output(stdout: &str, stderr: &str, exit_code: i32, duration_ms: u64) -> StepOutput {
    StepOutput {
        output: json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code,
        }),
        duration_ms,
        cost_usd: Decimal::ZERO,
        input_tokens: None,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        output_tokens: None,
        model: None,
        debug_messages: None,
        artifacts: StepArtifacts::default(),
        account_id: None,
        environment_id: None,
    }
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
    fn kind(&self) -> StepKind {
        StepKind::Shell
    }

    async fn execute(&self, _provider: &Arc<dyn AgentProvider>) -> Result<StepOutput, EngineError> {
        match self.log_sender {
            Some(ref sender) => self.execute_streaming(sender.clone()).await,
            None => self.execute_buffered().await,
        }
    }
}

impl ShellExecutor<'_> {
    /// The process to spawn: the program itself with its arguments in exec
    /// mode, `sh -c <command>` otherwise.
    fn command(&self) -> Command {
        match self.config.args {
            Some(ref args) => {
                let mut cmd = Command::new(&self.config.command);
                cmd.args(args);
                cmd
            }
            None => {
                let mut cmd = Command::new("sh");
                cmd.arg("-c").arg(&self.config.command);
                cmd
            }
        }
    }

    /// The command as shown in logs and timeout errors.
    fn display(&self) -> String {
        match self.config.args {
            Some(ref args) => [self.config.command.as_str()]
                .into_iter()
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
            None => self.config.command.clone(),
        }
    }

    /// Non-streaming execution via [`Shell::run()`].
    async fn execute_buffered(&self) -> Result<StepOutput, EngineError> {
        let start = Instant::now();

        let mut shell = match self.config.args {
            Some(ref args) => Shell::exec(
                &self.config.command,
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
            ),
            None => Shell::new(&self.config.command),
        };
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

        // `Shell::run` drops stdout on a non-zero exit, so the option needs its
        // own capture. A global dry run keeps going through `Shell::run`, which
        // short-circuits without spawning anything.
        let (stdout, stderr, exit_code) = if self.config.exit_code_as_output && !is_dry_run() {
            self.run_capturing().await?
        } else {
            let output = shell.run().await?;
            (
                output.stdout().to_string(),
                output.stderr().to_string(),
                output.exit_code(),
            )
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            step_kind = "shell",
            command = %self.display(),
            exit_code,
            duration_ms,
            "shell step completed"
        );

        self.record_metrics(duration_ms);

        Ok(build_output(&stdout, &stderr, exit_code, duration_ms))
    }

    /// Run the command to completion and keep stdout, stderr and the exit
    /// code whatever the code is. Used when `exit_code_as_output` is set.
    async fn run_capturing(&self) -> Result<(String, String, i32), EngineError> {
        let mut cmd = self.command();
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

        let child = cmd.spawn().map_err(|e| {
            EngineError::Operation(OperationError::Shell {
                exit_code: -1,
                stderr: format!("failed to spawn shell: {e}"),
            })
        })?;

        let timeout_dur = self
            .config
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_SHELL_TIMEOUT);

        let output = match tokio::time::timeout(timeout_dur, child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(EngineError::Operation(OperationError::Shell {
                    exit_code: -1,
                    stderr: format!("failed to wait for shell: {e}"),
                }));
            }
            Err(_) => {
                return Err(EngineError::Operation(OperationError::Timeout {
                    step: self.display(),
                    limit: timeout_dur,
                }));
            }
        };

        let exit_code = exit_code_of(output.status);
        let stdout = truncate_output(&output.stdout, "shell stdout");
        let stderr = truncate_output(&output.stderr, "shell stderr");
        Ok((stdout, stderr, exit_code))
    }

    /// Streaming execution: reads stdout/stderr line-by-line and forwards
    /// each line to the [`StepLogSender`] in real time.
    async fn execute_streaming(&self, sender: StepLogSender) -> Result<StepOutput, EngineError> {
        let start = Instant::now();

        let mut cmd = self.command();
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
                    step: self.display(),
                    limit: timeout_dur,
                }));
            }
        };

        let raw_stdout = stdout_task.await.unwrap_or_default();
        let raw_stderr = stderr_task.await.unwrap_or_default();

        let stdout = truncate_output(raw_stdout.as_bytes(), "shell stdout");
        let stderr = truncate_output(raw_stderr.as_bytes(), "shell stderr");

        let exit_code = exit_code_of(status);
        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            step_kind = "shell",
            command = %self.display(),
            exit_code,
            duration_ms,
            streaming = true,
            "shell step completed"
        );

        self.record_metrics(duration_ms);

        if exit_code != 0 && !self.config.exit_code_as_output {
            return Err(EngineError::Operation(OperationError::Shell {
                exit_code,
                stderr: stderr.clone(),
            }));
        }

        Ok(build_output(&stdout, &stderr, exit_code, duration_ms))
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
    use std::path::Path;

    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_core::providers::record_replay::RecordReplayProvider;
    use tempfile::tempdir;
    use uuid::Uuid;

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

    fn streaming_sender() -> StepLogSender {
        let (sender, _receiver) = crate::log_sender::channel();
        StepLogSender::new(
            sender,
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            "test".to_string(),
        )
    }

    #[tokio::test]
    async fn shell_exit_code_as_output_buffered_keeps_code_and_streams() {
        let config = ShellConfig::new("echo out; echo err >&2; exit 3").exit_code_as_output();
        let executor = ShellExecutor::new(&config);
        let provider = create_test_provider();

        let output = executor
            .execute(&provider)
            .await
            .expect("a non-zero exit is an output");
        assert_eq!(output.output["exit_code"], 3);
        assert_eq!(output.stdout().trim(), "out");
        assert_eq!(output.stderr().trim(), "err");
        assert!(!output.is_success());
        assert_eq!(output.exit_code(), Some(3));
    }

    #[tokio::test]
    async fn shell_exit_code_as_output_streaming_keeps_code() {
        let config = ShellConfig::new("echo out; echo err >&2; exit 3").exit_code_as_output();
        let executor = ShellExecutor::new(&config).with_log_sender(streaming_sender());
        let provider = create_test_provider();

        let output = executor
            .execute(&provider)
            .await
            .expect("a non-zero exit is an output");
        assert_eq!(output.output["exit_code"], 3);
        assert_eq!(output.stdout(), "out");
        assert_eq!(output.stderr(), "err");
        assert!(!output.is_success());
        assert_eq!(output.exit_code(), Some(3));
    }

    #[tokio::test]
    async fn shell_exit_code_as_output_zero_exit_is_success() {
        let config = ShellConfig::new("echo fine").exit_code_as_output();
        let provider = create_test_provider();

        let buffered = ShellExecutor::new(&config)
            .execute(&provider)
            .await
            .expect("exit 0 succeeds");
        assert!(buffered.is_success());
        assert_eq!(buffered.exit_code(), Some(0));

        let streaming = ShellExecutor::new(&config)
            .with_log_sender(streaming_sender())
            .execute(&provider)
            .await
            .expect("exit 0 succeeds");
        assert!(streaming.is_success());
    }

    #[tokio::test]
    async fn shell_exit_code_as_output_still_errors_on_timeout() {
        let config = ShellConfig::new("sleep 5")
            .timeout_secs(1)
            .exit_code_as_output();
        let provider = create_test_provider();

        let buffered = ShellExecutor::new(&config).execute(&provider).await;
        assert!(matches!(
            buffered,
            Err(EngineError::Operation(OperationError::Timeout { .. }))
        ));

        let streaming = ShellExecutor::new(&config)
            .with_log_sender(streaming_sender())
            .execute(&provider)
            .await;
        assert!(matches!(
            streaming,
            Err(EngineError::Operation(OperationError::Timeout { .. }))
        ));
    }

    #[tokio::test]
    async fn shell_without_exit_code_as_output_nonzero_still_errors() {
        let config = ShellConfig::new("echo out; exit 3");
        let provider = create_test_provider();

        let buffered = ShellExecutor::new(&config).execute(&provider).await;
        assert!(matches!(
            buffered,
            Err(EngineError::Operation(OperationError::Shell {
                exit_code: 3,
                ..
            }))
        ));

        let streaming = ShellExecutor::new(&config)
            .with_log_sender(streaming_sender())
            .execute(&provider)
            .await;
        assert!(matches!(
            streaming,
            Err(EngineError::Operation(OperationError::Shell {
                exit_code: 3,
                ..
            }))
        ));
    }

    /// Shell metacharacters that `sh -c` would act on.
    const HOSTILE_ARG: &str = "x'; echo injected; echo '$(echo sub) `echo tick` ${HOME} \\n";

    #[tokio::test]
    async fn exec_passes_each_argument_verbatim_buffered() {
        let config = ShellConfig::exec("printf", &["%s|%s", HOSTILE_ARG, "two words"]);
        let provider = create_test_provider();

        let output = ShellExecutor::new(&config)
            .execute(&provider)
            .await
            .expect("printf runs");

        assert_eq!(output.stdout(), format!("{HOSTILE_ARG}|two words"));
        assert_eq!(output.exit_code(), Some(0));
    }

    #[tokio::test]
    async fn exec_passes_each_argument_verbatim_streaming() {
        let config = ShellConfig::exec("printf", &["%s\n", HOSTILE_ARG]);
        let (sender, mut receiver) = crate::log_sender::channel();
        let step_sender =
            StepLogSender::new(sender, Uuid::now_v7(), Uuid::now_v7(), "test".to_string());
        let provider = create_test_provider();

        let output = ShellExecutor::new(&config)
            .with_log_sender(step_sender)
            .execute(&provider)
            .await
            .expect("printf runs");

        assert_eq!(output.stdout(), HOSTILE_ARG);
        let line = receiver.try_recv().expect("one streamed line");
        assert_eq!(line.line, HOSTILE_ARG);
    }

    #[tokio::test]
    async fn exec_with_exit_code_as_output_keeps_the_code_and_args() {
        let config =
            ShellConfig::exec("sh", &["-c", "printf %s \"$1\"; exit 3", "sh", HOSTILE_ARG])
                .exit_code_as_output();
        let provider = create_test_provider();

        let buffered = ShellExecutor::new(&config)
            .execute(&provider)
            .await
            .expect("a non-zero exit is an output");
        assert_eq!(buffered.exit_code(), Some(3));
        assert_eq!(buffered.stdout(), HOSTILE_ARG);

        let streaming = ShellExecutor::new(&config)
            .with_log_sender(streaming_sender())
            .execute(&provider)
            .await
            .expect("a non-zero exit is an output");
        assert_eq!(streaming.exit_code(), Some(3));
        assert_eq!(streaming.stdout(), HOSTILE_ARG);
    }

    #[tokio::test]
    async fn exec_nonzero_exit_returns_error() {
        let config = ShellConfig::exec("false", &[]);
        let provider = create_test_provider();

        let buffered = ShellExecutor::new(&config).execute(&provider).await;
        assert!(matches!(
            buffered,
            Err(EngineError::Operation(OperationError::Shell {
                exit_code: 1,
                ..
            }))
        ));

        let streaming = ShellExecutor::new(&config)
            .with_log_sender(streaming_sender())
            .execute(&provider)
            .await;
        assert!(matches!(
            streaming,
            Err(EngineError::Operation(OperationError::Shell {
                exit_code: 1,
                ..
            }))
        ));
    }

    #[tokio::test]
    async fn exec_of_a_missing_program_is_a_spawn_error() {
        let config = ShellConfig::exec("ironflow-no-such-program", &["arg"]);
        let provider = create_test_provider();

        for result in [
            ShellExecutor::new(&config).execute(&provider).await,
            ShellExecutor::new(&config)
                .with_log_sender(streaming_sender())
                .execute(&provider)
                .await,
            ShellExecutor::new(&config.clone().exit_code_as_output())
                .execute(&provider)
                .await,
        ] {
            match result {
                Err(EngineError::Operation(OperationError::Shell { exit_code, stderr })) => {
                    assert_eq!(exit_code, -1);
                    assert!(stderr.starts_with("failed to spawn"), "stderr: {stderr}");
                }
                other => panic!("expected a spawn error, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn exec_honours_dir_and_env() {
        let dir = tempdir().expect("temp dir");
        let config = ShellConfig::exec("sh", &["-c", "pwd; printf %s \"$GREETING\""])
            .dir(dir.path().to_str().expect("utf-8 path"))
            .env("GREETING", "hi; echo no");
        let provider = create_test_provider();

        let output = ShellExecutor::new(&config)
            .execute(&provider)
            .await
            .expect("sh runs");

        let canonical = dir.path().canonicalize().expect("canonical dir");
        let mut lines = output.stdout().lines();
        assert_eq!(
            Path::new(lines.next().expect("pwd line"))
                .canonicalize()
                .expect("canonical pwd"),
            canonical
        );
        assert_eq!(lines.next(), Some("hi; echo no"));
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
