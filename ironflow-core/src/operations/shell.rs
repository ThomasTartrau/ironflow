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
/// Default timeout for shell commands (5 minutes).
const DEFAULT_SHELL_TIMEOUT: Duration = Duration::from_secs(300);

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
