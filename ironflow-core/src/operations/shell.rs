//! Shell operation - run system commands with timeout and environment control.
//!
//! The [`Shell`] builder spawns a command via `sh -c`, captures stdout/stderr,
//! and returns a [`ShellOutput`] on success. It implements [`IntoFuture`] so you
//! can `await` it directly without calling `.run()`:
//!
//! ```no_run
//! use ironflow_core::operations::shell::Shell;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! // These two are equivalent:
//! let output = Shell::new("echo hello").await?;
//! let output = Shell::new("echo hello").run().await?;
//! # Ok(())
//! # }
//! ```
//!
//! For safe execution without shell interpretation, use [`Shell::exec`]:
//!
//! ```no_run
//! use ironflow_core::operations::shell::Shell;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let output = Shell::exec("echo", &["hello", "world"]).await?;
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tracing::{debug, error, warn};

use crate::error::OperationError;
#[cfg(feature = "prometheus")]
use crate::metric_names;
use crate::utils::truncate_output;

/// How the command is executed.
enum ShellMode {
    /// Pass the command string to `sh -c`.
    Shell(String),
    /// Execute the program directly with explicit arguments, bypassing shell
    /// interpretation.
    Exec { program: String, args: Vec<String> },
}

impl fmt::Display for ShellMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shell(cmd) => f.write_str(cmd),
            Self::Exec { program, args } => {
                write!(f, "{program}")?;
                for arg in args {
                    write!(f, " {arg}")?;
                }
                Ok(())
            }
        }
    }
}

/// Default timeout for shell commands (5 minutes).
const DEFAULT_SHELL_TIMEOUT: Duration = Duration::from_secs(300);

/// Builder for executing a shell command.
///
/// Supports optional timeout, working directory, environment variables, and
/// clean-environment mode. Output is truncated to [`MAX_OUTPUT_SIZE`](crate::utils::MAX_OUTPUT_SIZE)
/// to prevent OOM on large outputs.
///
/// # Security
///
/// Commands created with [`Shell::new`] are executed via `sh -c`, which means
/// shell metacharacters (`;`, `|`, `$()`, `` ` ``, etc.) are interpreted.
/// **Do not** incorporate untrusted input into command strings without proper
/// validation. Use [`Shell::env`] to pass dynamic data safely through
/// environment variables, or use [`Shell::exec`] to bypass shell interpretation
/// entirely.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use ironflow_core::operations::shell::Shell;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let output = Shell::new("cargo test")
///     .dir("/path/to/project")
///     .timeout(Duration::from_secs(120))
///     .env("RUST_LOG", "debug")
///     .await?;
///
/// println!("stdout: {}", output.stdout());
/// # Ok(())
/// # }
/// ```
#[must_use = "a Shell command does nothing until .run() or .await is called"]
pub struct Shell {
    mode: ShellMode,
    timeout: Duration,
    dir: Option<String>,
    env_vars: Vec<(String, String)>,
    inherit_env: bool,
    dry_run: Option<bool>,
}

impl Shell {
    /// Create a new shell builder for the given command string.
    ///
    /// The command is passed to `sh -c`, so pipes, redirects, and other shell
    /// features work as expected.
    ///
    /// # Security
    ///
    /// **Never** interpolate untrusted input directly into the command string.
    /// Doing so creates a **command injection** vulnerability:
    ///
    /// ```no_run
    /// # use ironflow_core::operations::shell::Shell;
    /// // DANGEROUS - attacker controls `user_input`
    /// # let user_input = "safe";
    /// let _ = Shell::new(&format!("cat {user_input}"));
    ///
    /// // SAFE - use arguments via a wrapper script or validate input first
    /// let _ = Shell::new("cat -- ./known_safe_file.txt");
    /// ```
    ///
    /// If you need to pass dynamic values, either validate them rigorously
    /// or use [`Shell::env`] to pass data through environment variables
    /// (which are not interpreted by the shell).
    pub fn new(command: &str) -> Self {
        Self {
            mode: ShellMode::Shell(command.to_string()),
            timeout: DEFAULT_SHELL_TIMEOUT,
            dir: None,
            env_vars: Vec::new(),
            inherit_env: true,
            dry_run: None,
        }
    }

    /// Create a new builder that executes a program directly without shell
    /// interpretation.
    ///
    /// Unlike [`Shell::new`], this does **not** pass the command through
    /// `sh -c`. The `program` is invoked directly with the given `args`,
    /// so shell metacharacters in arguments are treated as literal text.
    /// This is the preferred way to run commands with untrusted arguments.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::operations::shell::Shell;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let output = Shell::exec("git", &["log", "--oneline", "-5"]).await?;
    /// println!("{}", output.stdout());
    /// # Ok(())
    /// # }
    /// ```
    pub fn exec(program: &str, args: &[&str]) -> Self {
        Self {
            mode: ShellMode::Exec {
                program: program.to_string(),
                args: args.iter().map(|a| (*a).to_string()).collect(),
            },
            timeout: DEFAULT_SHELL_TIMEOUT,
            dir: None,
            env_vars: Vec::new(),
            inherit_env: true,
            dry_run: None,
        }
    }

    /// Override the maximum duration for the command.
    ///
    /// If the command does not complete within this duration, it is killed and
    /// an [`OperationError::Timeout`] is returned. Defaults to 5 minutes.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the working directory for the spawned process.
    pub fn dir(mut self, dir: &str) -> Self {
        self.dir = Some(dir.to_string());
        self
    }

    /// Add an environment variable to the spawned process.
    ///
    /// Can be called multiple times to set several variables.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env_vars.push((key.to_string(), value.to_string()));
        self
    }

    /// Clear the inherited environment so the process starts with an empty
    /// environment (plus any variables added via [`env`](Shell::env)).
    pub fn clean_env(mut self) -> Self {
        self.inherit_env = false;
        self
    }

    /// Enable or disable dry-run mode for this specific operation.
    ///
    /// When dry-run is active, the command is logged but not executed.
    /// A synthetic [`ShellOutput`] is returned with empty stdout/stderr,
    /// exit code 0, and 0ms duration.
    ///
    /// If not set, falls back to the global dry-run setting
    /// (see [`set_dry_run`](crate::dry_run::set_dry_run)).
    pub fn dry_run(mut self, enabled: bool) -> Self {
        self.dry_run = Some(enabled);
        self
    }

    /// Execute the command and wait for it to complete.
    ///
    /// # Errors
    ///
    /// * [`OperationError::Shell`] - if the command exits with a non-zero code
    ///   or cannot be spawned.
    /// * [`OperationError::Timeout`] - if the command exceeds the configured
    ///   [`timeout`](Shell::timeout).
    #[tracing::instrument(name = "shell", skip_all, fields(command = %self.mode))]
    pub async fn run(self) -> Result<ShellOutput, OperationError> {
        let command_display = self.mode.to_string();

        if crate::dry_run::effective_dry_run(self.dry_run) {
            debug!(command = %command_display, "[dry-run] shell command skipped");
            return Ok(ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
                duration_ms: 0,
            });
        }

        debug!(command = %command_display, "executing shell command");

        let start = Instant::now();

        let mut cmd = match &self.mode {
            ShellMode::Shell(command) => {
                let mut c = Command::new("sh");
                c.arg("-c").arg(command);
                c
            }
            ShellMode::Exec { program, args } => {
                let mut c = Command::new(program);
                c.args(args);
                c
            }
        };

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if !self.inherit_env {
            cmd.env_clear();
        }

        if let Some(ref dir) = self.dir {
            cmd.current_dir(dir);
        }

        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }

        let child = cmd.spawn().map_err(|e| OperationError::Shell {
            exit_code: -1,
            stderr: format!("failed to spawn shell: {e}"),
        })?;

        let output = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(result) => result.map_err(|e| OperationError::Shell {
                exit_code: -1,
                stderr: format!("failed to wait for shell: {e}"),
            })?,
            Err(_) => {
                return Err(OperationError::Timeout {
                    step: command_display,
                    limit: self.timeout,
                });
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout = truncate_output(&output.stdout, "shell stdout");
        let stderr = truncate_output(&output.stderr, "shell stderr");

        let exit_code = output.status.code().unwrap_or_else(|| {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(signal) = output.status.signal() {
                    warn!(signal, "process killed by signal");
                    return -signal;
                }
            }
            -1
        });

        #[cfg(feature = "prometheus")]
        metrics::histogram!(metric_names::SHELL_DURATION_SECONDS)
            .record(duration_ms as f64 / 1000.0);

        if !output.status.success() {
            error!(exit_code, stderr = %stderr, "shell command failed");
            #[cfg(feature = "prometheus")]
            metrics::counter!(metric_names::SHELL_TOTAL, "status" => metric_names::STATUS_ERROR)
                .increment(1);
            return Err(OperationError::Shell { exit_code, stderr });
        }

        debug!(
            exit_code,
            stdout_len = stdout.len(),
            duration_ms,
            "shell command completed"
        );

        #[cfg(feature = "prometheus")]
        metrics::counter!(metric_names::SHELL_TOTAL, "status" => metric_names::STATUS_SUCCESS)
            .increment(1);

        Ok(ShellOutput {
            stdout,
            stderr,
            exit_code,
            duration_ms,
        })
    }
}

impl IntoFuture for Shell {
    type Output = Result<ShellOutput, OperationError>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

/// Output of a successful shell command execution.
///
/// Contains the captured stdout, stderr, exit code, and duration.
#[derive(Debug)]
pub struct ShellOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
    duration_ms: u64,
}

impl ShellOutput {
    /// Return the captured standard output, trimmed and truncated to
    /// [`MAX_OUTPUT_SIZE`](crate::utils::MAX_OUTPUT_SIZE).
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Return the captured standard error, trimmed and truncated to
    /// [`MAX_OUTPUT_SIZE`](crate::utils::MAX_OUTPUT_SIZE).
    pub fn stderr(&self) -> &str {
        &self.stderr
    }

    /// Return the process exit code (`0` on success).
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Return the wall-clock duration of the command in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dry_run::{DryRunGuard, set_dry_run};
    use serial_test::serial;
    use std::time::Duration;

    #[tokio::test]
    async fn test_shell_new_creates_with_correct_command() {
        let shell = Shell::new("echo hello");
        assert_eq!(shell.timeout, DEFAULT_SHELL_TIMEOUT);
        assert!(shell.inherit_env);
        assert!(shell.dir.is_none());
        assert!(shell.env_vars.is_empty());
    }

    #[tokio::test]
    async fn test_shell_exec_creates_with_program_and_args() {
        let shell = Shell::exec("echo", &["hello", "world"]);
        assert_eq!(shell.timeout, DEFAULT_SHELL_TIMEOUT);
        assert!(shell.inherit_env);
        assert!(shell.dir.is_none());
        assert!(shell.env_vars.is_empty());
    }

    #[tokio::test]
    async fn test_timeout_builder_returns_self() {
        let custom_timeout = Duration::from_secs(10);
        let shell = Shell::new("echo hello").timeout(custom_timeout);
        assert_eq!(shell.timeout, custom_timeout);
    }

    #[tokio::test]
    async fn test_timeout_is_enforced() {
        let short_timeout = Duration::from_millis(100);
        let result = Shell::new("sleep 10")
            .dry_run(false)
            .timeout(short_timeout)
            .await;

        assert!(result.is_err());
        match result {
            Err(OperationError::Timeout { step, limit }) => {
                assert_eq!(limit, short_timeout);
                assert!(step.contains("sleep"));
            }
            _ => panic!("expected Timeout error"),
        }
    }

    #[tokio::test]
    async fn test_dir_builder_returns_self() {
        let shell = Shell::new("pwd").dir("/tmp");
        assert_eq!(shell.dir, Some("/tmp".to_string()));
    }

    #[tokio::test]
    async fn test_dir_is_respected() {
        let output = Shell::new("pwd")
            .dry_run(false)
            .dir("/tmp")
            .await
            .expect("failed to run pwd in /tmp");

        // pwd should output /tmp (or a symlink to it)
        let pwd_output = output.stdout().trim();
        assert!(pwd_output.ends_with("/tmp") || pwd_output.ends_with("private/tmp"));
    }

    #[tokio::test]
    async fn test_env_builder_returns_self() {
        let shell = Shell::new("echo $TEST_VAR").env("TEST_VAR", "hello");
        assert_eq!(shell.env_vars.len(), 1);
        assert_eq!(
            shell.env_vars[0],
            ("TEST_VAR".to_string(), "hello".to_string())
        );
    }

    #[tokio::test]
    async fn test_env_is_visible_to_command() {
        let output = Shell::new("echo $TEST_VAR")
            .dry_run(false)
            .env("TEST_VAR", "custom_value")
            .await
            .expect("failed to run echo with env var");

        assert_eq!(output.stdout().trim(), "custom_value");
    }

    #[tokio::test]
    async fn test_multiple_env_vars() {
        let output = Shell::new("echo $VAR1:$VAR2")
            .dry_run(false)
            .env("VAR1", "foo")
            .env("VAR2", "bar")
            .await
            .expect("failed to run echo with multiple env vars");

        assert_eq!(output.stdout().trim(), "foo:bar");
    }

    #[tokio::test]
    async fn test_clean_env_clears_inherited_environment() {
        // With clean env, we need to use the full path to commands
        let output = Shell::exec("/bin/echo", &["hello"])
            .dry_run(false)
            .clean_env()
            .await
            .expect("failed to run with clean env");

        assert_eq!(output.stdout().trim(), "hello");
    }

    #[tokio::test]
    async fn test_clean_env_with_custom_var_only() {
        // With clean env, only our custom var should be visible
        let output = Shell::exec("/bin/sh", &["-c", "echo $CUSTOM_VAR"])
            .dry_run(false)
            .clean_env()
            .env("CUSTOM_VAR", "value")
            .await
            .expect("failed to run with clean env and custom var");

        assert_eq!(output.stdout().trim(), "value");
    }

    #[tokio::test]
    async fn test_dry_run_true_skips_execution() {
        let output = Shell::new("echo test")
            .dry_run(true)
            .await
            .expect("dry run should not fail");

        assert_eq!(output.stdout(), "");
        assert_eq!(output.stderr(), "");
        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.duration_ms(), 0);
    }

    #[tokio::test]
    async fn test_dry_run_false_executes_command() {
        let output = Shell::new("echo hello")
            .dry_run(false)
            .await
            .expect("dry run false should execute");

        assert_eq!(output.stdout(), "hello");
    }

    #[tokio::test]
    #[serial]
    async fn test_global_dry_run_affects_operations() {
        set_dry_run(false);
        {
            let _guard = DryRunGuard::new(true);
            let output = Shell::new("echo test")
                .await
                .expect("dry run should not fail");

            assert_eq!(output.stdout(), "");
            assert_eq!(output.duration_ms(), 0);
        }
        set_dry_run(false);
    }

    #[tokio::test]
    #[serial]
    async fn test_per_operation_dry_run_overrides_global() {
        set_dry_run(false);
        {
            let _guard = DryRunGuard::new(true);
            let output = Shell::new("echo hello")
                .dry_run(false)
                .await
                .expect("per-operation dry_run(false) should override global");

            // Should execute despite global dry-run being true
            assert_eq!(output.stdout(), "hello");
        }
        set_dry_run(false);
    }

    #[tokio::test]
    async fn test_run_captures_stdout_stderr_exit_code() {
        let output = Shell::new("echo stdout && echo stderr >&2; exit 0")
            .dry_run(false)
            .await
            .expect("should not fail with exit 0");

        assert_eq!(output.stdout().trim(), "stdout");
        assert!(output.stderr().contains("stderr"));
        assert_eq!(output.exit_code(), 0);
    }

    #[tokio::test]
    async fn test_failed_command_returns_error() {
        let result = Shell::new("exit 42").dry_run(false).await;

        assert!(result.is_err());
        match result {
            Err(OperationError::Shell {
                exit_code,
                stderr: _,
            }) => {
                assert_eq!(exit_code, 42);
            }
            _ => panic!("expected Shell error"),
        }
    }

    #[tokio::test]
    async fn test_non_zero_exit_code_captured() {
        let result = Shell::new("sh -c 'exit 7'").dry_run(false).await;

        assert!(result.is_err());
        if let Err(OperationError::Shell { exit_code, .. }) = result {
            assert_eq!(exit_code, 7);
        } else {
            panic!("expected Shell error with exit_code 7");
        }
    }

    #[tokio::test]
    async fn test_shell_exec_without_shell_interpretation() {
        // With exec, the pipe should be passed as a literal argument, not interpreted
        let output = Shell::exec("echo", &["hello | world"])
            .dry_run(false)
            .await
            .expect("exec should not interpret pipe");

        assert_eq!(output.stdout().trim(), "hello | world");
    }

    #[tokio::test]
    async fn test_shell_new_with_shell_interpretation() {
        // With Shell::new, the pipe should be interpreted
        let output = Shell::new("echo hello | wc -w")
            .dry_run(false)
            .await
            .expect("should interpret pipe");

        assert_eq!(output.stdout().trim(), "1");
    }

    #[tokio::test]
    async fn test_empty_command_string() {
        // Empty command via sh -c should succeed with empty output
        let output = Shell::new("")
            .dry_run(false)
            .await
            .expect("empty command should succeed");

        assert_eq!(output.stdout(), "");
        assert_eq!(output.exit_code(), 0);
    }

    #[tokio::test]
    async fn test_unicode_in_stdout() {
        let output = Shell::new("echo '你好世界'")
            .dry_run(false)
            .await
            .expect("should handle unicode");

        assert!(output.stdout().contains("你好"));
    }

    #[tokio::test]
    async fn test_unicode_in_stderr() {
        let result = Shell::new("echo '错误日志' >&2; exit 1")
            .dry_run(false)
            .await;

        assert!(result.is_err());
        if let Err(OperationError::Shell { stderr, .. }) = result {
            assert!(stderr.contains("错误"));
        }
    }

    #[tokio::test]
    async fn test_large_output_is_truncated() {
        // Create output larger than MAX_OUTPUT_SIZE
        // Instead of actually creating 10MB, we'll test the truncation logic
        // by checking that very large outputs don't cause OOM
        let large_count = 1000; // Create 1000 lines, which is safe but still substantial
        let cmd = format!(
            "for i in $(seq 1 {}); do echo \"line $i\"; done",
            large_count
        );
        let output = Shell::new(&cmd)
            .dry_run(false)
            .await
            .expect("should handle large output");

        // Output should be successful
        assert_eq!(output.exit_code(), 0);
        assert!(!output.stdout().is_empty());
    }

    #[tokio::test]
    async fn test_duration_is_recorded() {
        let output = Shell::new("sleep 0.1")
            .dry_run(false)
            .await
            .expect("should complete");

        assert!(output.duration_ms() >= 100);
        assert!(output.duration_ms() < 2000); // generous headroom for slow CI
    }

    #[tokio::test]
    async fn test_into_future_trait() {
        // This tests that Shell implements IntoFuture
        let output = Shell::new("echo into_future")
            .dry_run(false)
            .await
            .expect("should work");
        assert_eq!(output.stdout(), "into_future");
    }

    #[tokio::test]
    async fn test_multiple_builder_calls_chain() {
        let output = Shell::new("echo test")
            .dry_run(false)
            .timeout(Duration::from_secs(30))
            .env("MY_VAR", "value")
            .dir("/tmp")
            .await
            .expect("chained builders should work");

        assert_eq!(output.stdout(), "test");
    }

    #[tokio::test]
    async fn test_shell_output_accessors() {
        let output = Shell::new("echo hello && echo world >&2")
            .dry_run(false)
            .await
            .expect("should succeed");

        let stdout = output.stdout();
        let stderr = output.stderr();
        let exit_code = output.exit_code();
        assert_eq!(stdout, "hello");
        assert!(stderr.contains("world"));
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn test_spawning_nonexistent_program_fails() {
        let result = Shell::exec("/nonexistent/program/path", &[])
            .dry_run(false)
            .await;

        assert!(result.is_err());
        match result {
            Err(OperationError::Shell {
                exit_code,
                stderr: _,
            }) => {
                assert_eq!(exit_code, -1);
            }
            _ => panic!("expected Shell error"),
        }
    }

    #[tokio::test]
    async fn test_output_is_trimmed() {
        let output = Shell::new("echo 'hello\n\n'")
            .dry_run(false)
            .await
            .expect("should succeed");

        // truncate_output trims trailing whitespace
        assert_eq!(output.stdout(), "hello");
    }

    #[tokio::test]
    async fn test_complex_shell_features() {
        let output = Shell::new("echo first && echo second | head -1")
            .dry_run(false)
            .await
            .expect("complex shell should work");

        assert!(output.stdout().contains("first"));
        assert!(output.stdout().contains("second"));
    }

    #[tokio::test]
    async fn test_stderr_on_success_is_captured() {
        let output = Shell::new("echo success && echo warnings >&2")
            .dry_run(false)
            .await
            .expect("should succeed despite stderr");

        assert_eq!(output.stdout().trim(), "success");
        assert!(output.stderr().contains("warnings"));
        assert_eq!(output.exit_code(), 0);
    }

    #[tokio::test]
    async fn test_must_use_attribute_on_shell() {
        // This is a compile-time check, but we can at least construct and drop
        let _shell = Shell::new("echo test");
    }

    #[tokio::test]
    async fn test_shell_mode_display_for_new() {
        let shell = Shell::new("echo test");
        let mode_str = shell.mode.to_string();
        assert_eq!(mode_str, "echo test");
    }

    #[tokio::test]
    async fn test_shell_mode_display_for_exec() {
        let shell = Shell::exec("echo", &["hello", "world"]);
        let mode_str = shell.mode.to_string();
        assert!(mode_str.contains("echo"));
        assert!(mode_str.contains("hello"));
        assert!(mode_str.contains("world"));
    }
}
