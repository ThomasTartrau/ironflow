//! Serializable step configurations — one per operation type.
//!
//! These types mirror the builder options in [`ironflow_core`] operations but
//! are fully serializable, allowing them to be stored as JSON in the database
//! and reconstructed by the executor at runtime.

mod agent;
mod approval;
mod artifact;
mod http;
mod shell;
mod workflow;

pub use agent::AgentStepConfig;
pub use approval::ApprovalConfig;
pub use artifact::{ArtifactInput, ArtifactOutput};
pub use http::HttpConfig;
pub use shell::ShellConfig;
pub use workflow::WorkflowStepConfig;

use ironflow_core::retry::RetryPolicy;
use ironflow_store::entities::StepKind;
use serde::{Deserialize, Serialize};

/// A serializable step configuration, wrapping one of the operation-specific configs.
///
/// Stored as JSON in the `steps.input` column and reconstructed by the
/// executor at runtime.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{StepConfig, ShellConfig};
///
/// let config = StepConfig::Shell(ShellConfig::new("echo hello"));
/// let json = serde_json::to_string(&config).unwrap();
/// assert!(json.contains("echo hello"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepConfig {
    /// A shell command step.
    Shell(ShellConfig),
    /// An HTTP request step.
    Http(HttpConfig),
    /// An AI agent step.
    Agent(AgentStepConfig),
    /// A sub-workflow invocation step.
    Workflow(WorkflowStepConfig),
    /// A human approval gate step.
    Approval(ApprovalConfig),
}

impl StepConfig {
    /// Whether this step is allowed to fail without stopping the run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{StepConfig, ShellConfig};
    ///
    /// let config = StepConfig::Shell(ShellConfig::new("cargo clippy").allow_failure());
    /// assert!(config.allow_failure());
    ///
    /// let config = StepConfig::Shell(ShellConfig::new("cargo build"));
    /// assert!(!config.allow_failure());
    /// ```
    pub fn allow_failure(&self) -> bool {
        match self {
            StepConfig::Shell(c) => c.allow_failure,
            StepConfig::Http(c) => c.allow_failure,
            StepConfig::Agent(c) => c.allow_failure,
            StepConfig::Workflow(_) | StepConfig::Approval(_) => false,
        }
    }

    /// Get the step-level retry policy, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::retry::RetryPolicy;
    /// use ironflow_engine::config::{StepConfig, ShellConfig};
    ///
    /// let config = StepConfig::Shell(ShellConfig::new("echo test").retry_policy(RetryPolicy::new(3)));
    /// assert!(config.retry().is_some());
    ///
    /// let config = StepConfig::Shell(ShellConfig::new("echo test"));
    /// assert!(config.retry().is_none());
    /// ```
    pub fn retry(&self) -> Option<&RetryPolicy> {
        match self {
            StepConfig::Shell(c) => c.retry.as_ref(),
            StepConfig::Http(c) => c.retry.as_ref(),
            StepConfig::Agent(c) => c.retry.as_ref(),
            StepConfig::Workflow(c) => c.retry.as_ref(),
            StepConfig::Approval(_) => None,
        }
    }

    /// Get the kind of step this configuration represents.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{StepConfig, ShellConfig};
    /// use ironflow_store::entities::StepKind;
    ///
    /// let config = StepConfig::Shell(ShellConfig::new("echo test"));
    /// assert_eq!(config.kind(), StepKind::Shell);
    /// ```
    pub fn kind(&self) -> StepKind {
        match self {
            StepConfig::Shell(_) => StepKind::Shell,
            StepConfig::Http(_) => StepKind::Http,
            StepConfig::Agent(_) => StepKind::Agent,
            StepConfig::Workflow(_) => StepKind::Workflow,
            StepConfig::Approval(_) => StepKind::Approval,
        }
    }
}

impl From<ShellConfig> for StepConfig {
    fn from(c: ShellConfig) -> Self {
        StepConfig::Shell(c)
    }
}

impl From<HttpConfig> for StepConfig {
    fn from(c: HttpConfig) -> Self {
        StepConfig::Http(c)
    }
}

impl From<AgentStepConfig> for StepConfig {
    fn from(c: AgentStepConfig) -> Self {
        StepConfig::Agent(c)
    }
}

#[cfg(test)]
mod tests {
    use ironflow_core::retry::RetryPolicy;

    use super::*;

    #[test]
    fn retry_accessor_returns_policy_for_each_variant() {
        let shell = StepConfig::Shell(ShellConfig::new("echo").retry_policy(RetryPolicy::new(2)));
        assert_eq!(shell.retry().unwrap().max_retries(), 2);

        let http = StepConfig::Http(HttpConfig::get("http://x").retry_policy(RetryPolicy::new(3)));
        assert_eq!(http.retry().unwrap().max_retries(), 3);

        let workflow = StepConfig::Workflow(
            WorkflowStepConfig::new("build", serde_json::json!({}))
                .retry_policy(RetryPolicy::new(4)),
        );
        assert_eq!(workflow.retry().unwrap().max_retries(), 4);

        let agent = StepConfig::Agent(AgentStepConfig::new("test"));
        assert!(agent.retry().is_none());

        let approval = StepConfig::Approval(ApprovalConfig::new("ok?"));
        assert!(approval.retry().is_none());
    }

    #[test]
    fn serde_roundtrip() {
        let configs = vec![
            StepConfig::Shell(ShellConfig::new("echo test")),
            StepConfig::Http(HttpConfig::get("http://example.com")),
            StepConfig::Agent(AgentStepConfig::new("summarize")),
            StepConfig::Workflow(WorkflowStepConfig::new("build", serde_json::json!({}))),
            StepConfig::Approval(ApprovalConfig::new("Deploy to production?")),
        ];

        for config in configs {
            let json = serde_json::to_string(&config).expect("serialize");
            let back: StepConfig = serde_json::from_str(&json).expect("deserialize");
            let json2 = serde_json::to_string(&back).expect("serialize2");
            assert_eq!(json, json2);
        }
    }
}
