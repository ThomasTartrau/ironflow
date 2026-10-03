//! [`ShellConfig`] — serializable configuration for a shell step.

use ironflow_core::retry::RetryPolicy;
use serde::{Deserialize, Serialize};

use super::artifact::{ArtifactInput, ArtifactOutput, ArtifactRef};

/// Serializable configuration for a shell step.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::ShellConfig;
///
/// let config = ShellConfig::new("cargo build --release")
///     .timeout_secs(300)
///     .dir("/app")
///     .output("target/report.html");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    /// The command line passed to `sh -c`, or the program to run when
    /// [`args`](Self::args) is set.
    pub command: String,
    /// Arguments of [`command`](Self::command) when it runs without a shell.
    ///
    /// `Some` means exec mode: `command` is spawned directly with these
    /// arguments, none of them is interpreted. `None` means `command` goes
    /// through `sh -c`. Set by [`ShellConfig::exec`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Timeout in seconds (default: 300).
    pub timeout_secs: Option<u64>,
    /// Working directory.
    pub dir: Option<String>,
    /// Environment variables to set.
    pub env: Vec<(String, String)>,
    /// If true, start with a clean environment.
    pub clean_env: bool,
    /// Files the step promises to produce, collected once it finishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<ArtifactOutput>,
    /// Artifacts of earlier steps to place in the working directory first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ArtifactInput>,
    /// When `true`, a failure of this step does not fail the run. The step is
    /// still marked `Failed` but execution continues and the run finishes with
    /// `RunStatus::Warning` instead of `Failed`.
    #[serde(default)]
    pub allow_failure: bool,
    /// When `true`, a non-zero exit code is a normal output instead of an
    /// error: the step is `Completed` and its output carries the real code.
    /// Timeout, spawn and input-preparation failures stay errors.
    #[serde(default)]
    pub exit_code_as_output: bool,
    /// Optional step-level retry policy. When set, a transient failure retries
    /// the step locally with exponential backoff before propagating the error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
}

impl ShellConfig {
    /// Create a new shell config with the given command.
    ///
    /// The command is passed to `sh -c`, so pipes, redirects and globs work.
    ///
    /// # Security
    ///
    /// Never build the command from data the workflow does not control (its
    /// input, a webhook payload, a human answer, an agent output): a quote or a
    /// `;` in that data runs arbitrary commands on the worker. Use
    /// [`ShellConfig::exec`] to pass such data as arguments, or
    /// [`env`](Self::env) to hand it to a script as a variable.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("echo hello");
    /// assert_eq!(config.command, "echo hello");
    /// ```
    pub fn new(command: &str) -> Self {
        Self {
            command: command.to_string(),
            args: None,
            timeout_secs: None,
            dir: None,
            env: Vec::new(),
            clean_env: false,
            outputs: Vec::new(),
            inputs: Vec::new(),
            allow_failure: false,
            exit_code_as_output: false,
            retry: None,
        }
    }

    /// Create a config that runs `program` directly, without a shell.
    ///
    /// Each argument reaches the program as is: quotes, `;`, `$(..)`,
    /// backticks and globs are plain text. This is the way to run a command
    /// built from untrusted data. The program is looked up in `PATH`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let name = "Ada'; rm -rf / #";
    /// let config = ShellConfig::exec("printf", &["Hello, %s!\n", name]);
    /// assert_eq!(config.command, "printf");
    /// assert_eq!(config.args.as_deref().map(<[String]>::len), Some(2));
    /// ```
    pub fn exec(program: &str, args: &[&str]) -> Self {
        Self {
            args: Some(args.iter().map(|arg| (*arg).to_string()).collect()),
            ..Self::new(program)
        }
    }

    /// Set the timeout in seconds.
    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Set the working directory.
    pub fn dir(mut self, dir: &str) -> Self {
        self.dir = Some(dir.to_string());
        self
    }

    /// Add an environment variable.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    /// Start with a clean environment (no inherited vars).
    pub fn clean_env(mut self) -> Self {
        self.clean_env = true;
        self
    }

    /// Declare a file the step produces, typed from its name.
    ///
    /// `pattern` is a glob resolved against [`dir`](Self::dir). Every match is
    /// stored as an artifact named after the file. When the step succeeds and
    /// the pattern matches nothing, the step fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("cargo build").output("target/*.log");
    /// assert_eq!(config.outputs.len(), 1);
    /// ```
    pub fn output(mut self, pattern: &str) -> Self {
        self.outputs.push(ArtifactOutput::new(pattern));
        self
    }

    /// Declare a produced file with an explicit MIME type.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("./gen").output_typed("data", "application/json");
    /// assert_eq!(config.outputs[0].content_type.as_deref(), Some("application/json"));
    /// ```
    pub fn output_typed(mut self, pattern: &str, content_type: &str) -> Self {
        self.outputs
            .push(ArtifactOutput::typed(pattern, content_type));
        self
    }

    /// Consume an artifact produced by an earlier step of the same run.
    ///
    /// The handle comes from the producing step, see [`ArtifactRef`]. The
    /// artifact is written into the working directory under its own name
    /// before the command runs. Use [`input_at`](Self::input_at) to choose
    /// another path.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let build = ctx.shell("build", ShellConfig::new("./gen").output("report.html")).await?;
    /// let report = build.artifact("report.html")?;
    /// let config = ShellConfig::new("./publish").input(&report);
    /// assert_eq!(config.inputs[0].destination(), "report.html");
    /// # Ok(())
    /// # }
    /// ```
    pub fn input(mut self, artifact: &ArtifactRef) -> Self {
        self.inputs.push(ArtifactInput::from(artifact));
        self
    }

    /// Mark this step as allowed to fail without stopping the run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("cargo clippy").allow_failure();
    /// assert!(config.allow_failure);
    /// ```
    pub fn allow_failure(mut self) -> Self {
        self.allow_failure = true;
        self
    }

    /// Treat a non-zero exit code as data instead of a failure.
    ///
    /// The step is `Completed`: `StepOutput::is_success()` is `false` and
    /// `exit_code()` returns the real code, so the handler can branch on it
    /// (a conflicting `git merge`, red tests). The run is not degraded and
    /// [`allow_failure`](Self::allow_failure) is not triggered. A non-zero
    /// exit is no longer an error, so a retry policy does not retry it.
    /// Timeout, spawn failure and input-preparation failure remain errors and
    /// follow `allow_failure`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("git merge feature").exit_code_as_output();
    /// assert!(config.exit_code_as_output);
    /// ```
    pub fn exit_code_as_output(mut self) -> Self {
        self.exit_code_as_output = true;
        self
    }

    /// Set a step-level retry policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::retry::RetryPolicy;
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("curl http://api")
    ///     .retry_policy(RetryPolicy::new(3));
    /// assert!(config.retry.is_some());
    /// ```
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = Some(policy);
        self
    }

    /// Consume an artifact and write it to an explicit path.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let build = ctx.shell("build", ShellConfig::new("./gen").output("report.html")).await?;
    /// let report = build.artifact("report.html")?;
    /// let config = ShellConfig::new("./publish").input_at(&report, "in/r.html");
    /// assert_eq!(config.inputs[0].destination(), "in/r.html");
    /// # Ok(())
    /// # }
    /// ```
    pub fn input_at(mut self, artifact: &ArtifactRef, dest: &str) -> Self {
        self.inputs.push(ArtifactInput::from(artifact).at(dest));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder() {
        let config = ShellConfig::new("cargo test")
            .timeout_secs(60)
            .dir("/app")
            .env("RUST_LOG", "debug")
            .clean_env();

        assert_eq!(config.command, "cargo test");
        assert_eq!(config.timeout_secs, Some(60));
        assert_eq!(config.dir, Some("/app".to_string()));
        assert_eq!(
            config.env,
            vec![("RUST_LOG".to_string(), "debug".to_string())]
        );
        assert!(config.clean_env);
    }

    #[test]
    fn a_fresh_config_declares_no_artifact() {
        let config = ShellConfig::new("echo hi");
        assert!(config.outputs.is_empty());
        assert!(config.inputs.is_empty());
    }

    #[test]
    fn outputs_and_inputs_accumulate_in_declaration_order() {
        let config = ShellConfig::new("build")
            .output("a.txt")
            .output_typed("b", "text/csv")
            .input(&ArtifactRef::new("prev", "c.txt"))
            .input_at(&ArtifactRef::new("prev", "d.txt"), "in/d.txt");

        assert_eq!(config.outputs[0].pattern, "a.txt");
        assert_eq!(config.outputs[1].content_type.as_deref(), Some("text/csv"));
        assert_eq!(config.inputs[0], ArtifactInput::new("prev", "c.txt"));
        assert_eq!(config.inputs[0].destination(), "c.txt");
        assert_eq!(config.inputs[1].step, "prev");
        assert_eq!(config.inputs[1].destination(), "in/d.txt");
    }

    #[test]
    fn serde_omits_empty_artifact_declarations() {
        let json = serde_json::to_string(&ShellConfig::new("echo hi")).expect("serialize");
        assert!(!json.contains("outputs"));
        assert!(!json.contains("inputs"));
    }

    #[test]
    fn a_config_predating_artifacts_still_deserializes() {
        let config: ShellConfig = serde_json::from_str(
            r#"{"command":"echo hi","timeout_secs":null,"dir":null,"env":[],"clean_env":false}"#,
        )
        .expect("deserialize");

        assert!(config.outputs.is_empty());
        assert!(config.inputs.is_empty());
    }

    #[test]
    fn a_config_predating_retry_still_deserializes() {
        let config: ShellConfig = serde_json::from_str(
            r#"{"command":"echo hi","timeout_secs":null,"dir":null,"env":[],"clean_env":false,"allow_failure":false}"#,
        )
        .expect("deserialize");

        assert!(config.retry.is_none());
    }

    #[test]
    fn a_config_predating_exit_code_as_output_still_deserializes() {
        let config: ShellConfig = serde_json::from_str(
            r#"{"command":"echo hi","timeout_secs":null,"dir":null,"env":[],"clean_env":false,"allow_failure":false}"#,
        )
        .expect("deserialize");

        assert!(!config.exit_code_as_output);
    }

    #[test]
    fn exit_code_as_output_roundtrip() {
        assert!(!ShellConfig::new("x").exit_code_as_output);

        let config = ShellConfig::new("git merge x").exit_code_as_output();
        let json = serde_json::to_string(&config).expect("serialize");
        let back: ShellConfig = serde_json::from_str(&json).expect("deserialize");
        assert!(back.exit_code_as_output);
    }

    #[test]
    fn exec_keeps_the_program_and_each_argument_apart() {
        let config = ShellConfig::exec("printf", &["%s\n", "a b; rm -rf /"]);

        assert_eq!(config.command, "printf");
        assert_eq!(
            config.args,
            Some(vec!["%s\n".to_string(), "a b; rm -rf /".to_string()])
        );
    }

    #[test]
    fn exec_with_no_argument_is_still_exec_mode() {
        let config = ShellConfig::exec("true", &[]);
        assert_eq!(config.args, Some(Vec::new()));
    }

    #[test]
    fn new_runs_through_the_shell() {
        assert!(ShellConfig::new("echo hi").args.is_none());
    }

    #[test]
    fn exec_args_roundtrip_and_are_omitted_in_shell_mode() {
        let shell = serde_json::to_string(&ShellConfig::new("echo hi")).expect("serialize");
        assert!(!shell.contains("args"));

        let json =
            serde_json::to_string(&ShellConfig::exec("git", &["log", "-1"])).expect("serialize");
        let back: ShellConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.command, "git");
        assert_eq!(back.args, Some(vec!["log".to_string(), "-1".to_string()]));
    }

    #[test]
    fn a_config_predating_exec_still_deserializes_in_shell_mode() {
        let config: ShellConfig = serde_json::from_str(
            r#"{"command":"echo hi","timeout_secs":null,"dir":null,"env":[],"clean_env":false,"allow_failure":false}"#,
        )
        .expect("deserialize");

        assert!(config.args.is_none());
    }

    #[test]
    fn retry_policy_roundtrip() {
        use ironflow_core::retry::RetryPolicy;

        let config = ShellConfig::new("curl http://api").retry_policy(RetryPolicy::new(3));
        let json = serde_json::to_string(&config).expect("serialize");
        let back: ShellConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.retry.as_ref().unwrap().max_retries(), 3);
    }
}
