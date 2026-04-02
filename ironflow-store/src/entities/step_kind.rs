//! [`StepKind`] — the type of operation a step executes.

use serde::{Deserialize, Serialize};

/// The type of operation a step executes.
///
/// Built-in kinds cover the four core operation types. The [`Custom`](StepKind::Custom)
/// variant allows user-defined operations (e.g. GitLab, Gmail, Slack) that
/// implement the `Operation` trait from `ironflow-engine`.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::StepKind;
///
/// let kind = StepKind::Shell;
/// assert_eq!(kind.to_string(), "Shell");
///
/// let custom = StepKind::Custom("gitlab".to_string());
/// assert_eq!(custom.to_string(), "Custom(gitlab)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// A shell command.
    Shell,
    /// An HTTP request.
    Http,
    /// An AI agent invocation.
    Agent,
    /// A sub-workflow invocation.
    Workflow,
    /// A user-defined operation (e.g. `"gitlab"`, `"gmail"`, `"slack"`).
    Custom(String),
}

impl std::fmt::Display for StepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepKind::Shell => f.write_str("Shell"),
            StepKind::Http => f.write_str("Http"),
            StepKind::Agent => f.write_str("Agent"),
            StepKind::Workflow => f.write_str("Workflow"),
            StepKind::Custom(name) => write!(f, "Custom({name})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        assert_eq!(StepKind::Shell.to_string(), "Shell");
        assert_eq!(StepKind::Http.to_string(), "Http");
        assert_eq!(StepKind::Agent.to_string(), "Agent");
        assert_eq!(StepKind::Workflow.to_string(), "Workflow");
        assert_eq!(
            StepKind::Custom("gitlab".to_string()).to_string(),
            "Custom(gitlab)"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let kind = StepKind::Workflow;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"workflow\"");
        let back: StepKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, StepKind::Workflow);
    }

    #[test]
    fn serde_roundtrip_custom() {
        let kind = StepKind::Custom("gitlab".to_string());
        let json = serde_json::to_string(&kind).unwrap();
        let back: StepKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }
}
