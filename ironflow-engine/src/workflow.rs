//! Workflow definition and builder.
//!
//! A [`WorkflowDef`] is an immutable, serializable description of a workflow —
//! a named sequence of steps. Use the [`Workflow`] builder to create one.
//!
//! # Examples
//!
//! ```
//! use ironflow_engine::config::{ShellConfig, AgentStepConfig};
//! use ironflow_engine::workflow::Workflow;
//!
//! let workflow = Workflow::new("deploy")
//!     .shell("build", ShellConfig::new("cargo build --release"))
//!     .shell("test", ShellConfig::new("cargo test"))
//!     .agent("review", AgentStepConfig::new("Review the diff"))
//!     .build()
//!     .expect("valid workflow");
//!
//! assert_eq!(workflow.name, "deploy");
//! assert_eq!(workflow.steps.len(), 3);
//! ```

use serde::{Deserialize, Serialize};

use crate::config::{AgentStepConfig, HttpConfig, ShellConfig, StepConfig, WorkflowStepConfig};
use crate::error::EngineError;

// ---------------------------------------------------------------------------
// WorkflowDef (immutable definition)
// ---------------------------------------------------------------------------

/// An immutable workflow definition: a named sequence of steps.
///
/// Created via the [`Workflow`] builder. Can be serialized for storage.
///
/// # Examples
///
/// ```
/// use ironflow_engine::workflow::Workflow;
/// use ironflow_engine::config::ShellConfig;
///
/// let def = Workflow::new("ci").shell("test", ShellConfig::new("cargo test")).build().unwrap();
/// assert_eq!(def.name, "ci");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    /// Workflow name (used as the key for lookups and display).
    pub name: String,
    /// Ordered list of step definitions.
    pub steps: Vec<StepDef>,
}

/// A single step within a workflow definition.
///
/// Holds the step name and its serializable configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDef {
    /// Human-readable step name.
    pub name: String,
    /// The operation configuration for this step.
    pub config: StepConfig,
}

// ---------------------------------------------------------------------------
// Workflow builder
// ---------------------------------------------------------------------------

/// Builder for creating a [`WorkflowDef`].
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::ShellConfig;
/// use ironflow_engine::workflow::Workflow;
///
/// let def = Workflow::new("deploy")
///     .shell("build", ShellConfig::new("cargo build"))
///     .build()
///     .expect("valid workflow");
/// ```
#[must_use = "a Workflow builder does nothing until .build() is called"]
pub struct Workflow {
    name: String,
    steps: Vec<StepDef>,
}

impl Workflow {
    /// Start building a new workflow with the given name.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::workflow::Workflow;
    ///
    /// let builder = Workflow::new("my-pipeline");
    /// ```
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            steps: Vec::new(),
        }
    }

    /// Add a shell step.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::workflow::Workflow;
    ///
    /// let builder = Workflow::new("ci")
    ///     .shell("test", ShellConfig::new("cargo test"));
    /// ```
    pub fn shell(mut self, name: &str, config: ShellConfig) -> Self {
        self.steps.push(StepDef {
            name: name.to_string(),
            config: StepConfig::Shell(config),
        });
        self
    }

    /// Add an HTTP step.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HttpConfig;
    /// use ironflow_engine::workflow::Workflow;
    ///
    /// let builder = Workflow::new("notify")
    ///     .http("webhook", HttpConfig::post("https://hooks.example.com/notify"));
    /// ```
    pub fn http(mut self, name: &str, config: HttpConfig) -> Self {
        self.steps.push(StepDef {
            name: name.to_string(),
            config: StepConfig::Http(config),
        });
        self
    }

    /// Add an agent step.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::AgentStepConfig;
    /// use ironflow_engine::workflow::Workflow;
    ///
    /// let builder = Workflow::new("review")
    ///     .agent("review-code", AgentStepConfig::new("Review this PR"));
    /// ```
    pub fn agent(mut self, name: &str, config: AgentStepConfig) -> Self {
        self.steps.push(StepDef {
            name: name.to_string(),
            config: StepConfig::Agent(config),
        });
        self
    }

    /// Add a sub-workflow step.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::WorkflowStepConfig;
    /// use ironflow_engine::workflow::Workflow;
    /// use ironflow_engine::config::ShellConfig;
    /// use serde_json::json;
    ///
    /// let builder = Workflow::new("pipeline")
    ///     .shell("lint", ShellConfig::new("cargo clippy"))
    ///     .workflow("run-tests", WorkflowStepConfig::new("ci-test", json!({})));
    /// ```
    pub fn workflow(mut self, name: &str, config: WorkflowStepConfig) -> Self {
        self.steps.push(StepDef {
            name: name.to_string(),
            config: StepConfig::Workflow(config),
        });
        self
    }

    /// Add a step with an arbitrary [`StepConfig`].
    ///
    /// Use this when you have a pre-built `StepConfig` or need to add
    /// steps programmatically.
    pub fn step(mut self, name: &str, config: StepConfig) -> Self {
        self.steps.push(StepDef {
            name: name.to_string(),
            config,
        });
        self
    }

    /// Consume the builder and produce an immutable [`WorkflowDef`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::StepConfig`] if the name is empty/whitespace
    /// or no steps were added.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::workflow::Workflow;
    ///
    /// let def = Workflow::new("ci")
    ///     .shell("test", ShellConfig::new("cargo test"))
    ///     .build()
    ///     .expect("valid workflow");
    /// assert_eq!(def.steps.len(), 1);
    /// ```
    pub fn build(self) -> Result<WorkflowDef, EngineError> {
        if self.name.trim().is_empty() {
            return Err(EngineError::StepConfig(
                "workflow name must not be empty".into(),
            ));
        }
        if self.steps.is_empty() {
            return Err(EngineError::StepConfig(
                "workflow must have at least one step".into(),
            ));
        }

        Ok(WorkflowDef {
            name: self.name,
            steps: self.steps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_simple_workflow() {
        let def = Workflow::new("deploy")
            .shell("build", ShellConfig::new("cargo build"))
            .shell("test", ShellConfig::new("cargo test"))
            .build()
            .unwrap();

        assert_eq!(def.name, "deploy");
        assert_eq!(def.steps.len(), 2);
        assert_eq!(def.steps[0].name, "build");
        assert_eq!(def.steps[1].name, "test");
    }

    #[test]
    fn build_mixed_step_types() {
        let def = Workflow::new("pipeline")
            .shell("build", ShellConfig::new("cargo build"))
            .http("notify", HttpConfig::post("http://hooks.example.com"))
            .agent("review", AgentStepConfig::new("Review code"))
            .build()
            .unwrap();

        assert_eq!(def.steps.len(), 3);
        assert!(matches!(def.steps[0].config, StepConfig::Shell(_)));
        assert!(matches!(def.steps[1].config, StepConfig::Http(_)));
        assert!(matches!(def.steps[2].config, StepConfig::Agent(_)));
    }

    #[test]
    fn build_returns_error_on_empty_name() {
        let result = Workflow::new("  ")
            .shell("step", ShellConfig::new("echo"))
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn build_returns_error_on_no_steps() {
        let result = Workflow::new("empty").build();
        assert!(result.is_err());
    }

    #[test]
    fn workflow_def_serde_roundtrip() {
        let def = Workflow::new("test")
            .shell("s1", ShellConfig::new("echo hello"))
            .build()
            .unwrap();

        let json = serde_json::to_string(&def).expect("serialize");
        let back: WorkflowDef = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.name, "test");
        assert_eq!(back.steps.len(), 1);
    }

    #[test]
    fn step_method_with_step_config() {
        let config = StepConfig::Shell(ShellConfig::new("echo test"));
        let def = Workflow::new("test")
            .step("generic", config)
            .build()
            .unwrap();
        assert_eq!(def.steps.len(), 1);
    }
}
