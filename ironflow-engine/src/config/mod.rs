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

#[cfg(test)]
mod tests {
    use super::*;

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
