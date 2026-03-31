//! [`StepKind`] — the type of operation a step executes.

use serde::{Deserialize, Serialize};

/// The type of operation a step executes.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::StepKind;
///
/// let kind = StepKind::Shell;
/// assert_eq!(kind.to_string(), "Shell");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
}

impl std::fmt::Display for StepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepKind::Shell => f.write_str("Shell"),
            StepKind::Http => f.write_str("Http"),
            StepKind::Agent => f.write_str("Agent"),
            StepKind::Workflow => f.write_str("Workflow"),
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
    }

    #[test]
    fn serde_roundtrip() {
        let kind = StepKind::Workflow;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"workflow\"");
        let back: StepKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, StepKind::Workflow);
    }
}
