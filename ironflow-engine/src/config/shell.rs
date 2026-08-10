//! [`ShellConfig`] — serializable configuration for a shell step.

use serde::{Deserialize, Serialize};

use super::artifact::{ArtifactInput, ArtifactOutput};

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
    /// The shell command to execute.
    pub command: String,
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
    /// [`RunStatus::Warning`] instead of `Failed`.
    #[serde(default)]
    pub allow_failure: bool,
}

impl ShellConfig {
    /// Create a new shell config with the given command.
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
            timeout_secs: None,
            dir: None,
            env: Vec::new(),
            clean_env: false,
            outputs: Vec::new(),
            inputs: Vec::new(),
            allow_failure: false,
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
    /// It is written into the working directory under its own name before the
    /// command runs. Use [`input_at`](Self::input_at) to choose another path.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("./publish").input("build", "report.html");
    /// assert_eq!(config.inputs[0].destination(), "report.html");
    /// ```
    pub fn input(mut self, step: &str, name: &str) -> Self {
        self.inputs.push(ArtifactInput::new(step, name));
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

    /// Consume an artifact and write it to an explicit path.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("./publish").input_at("build", "report.html", "in/r.html");
    /// assert_eq!(config.inputs[0].destination(), "in/r.html");
    /// ```
    pub fn input_at(mut self, step: &str, name: &str, dest: &str) -> Self {
        self.inputs.push(ArtifactInput::new(step, name).at(dest));
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
            .input("prev", "c.txt")
            .input_at("prev", "d.txt", "in/d.txt");

        assert_eq!(config.outputs[0].pattern, "a.txt");
        assert_eq!(config.outputs[1].content_type.as_deref(), Some("text/csv"));
        assert_eq!(config.inputs[0].destination(), "c.txt");
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
}
