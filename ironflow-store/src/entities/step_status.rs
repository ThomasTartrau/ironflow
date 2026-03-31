//! [`StepStatus`] — lifecycle states for a workflow step.

use serde::{Deserialize, Serialize};

/// Status of an individual step within a run.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::StepStatus;
///
/// assert!(!StepStatus::Pending.is_terminal());
/// assert!(StepStatus::Completed.is_terminal());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Waiting to execute.
    Pending,
    /// Currently executing.
    Running,
    /// Executed successfully.
    Completed,
    /// Execution failed.
    Failed,
    /// Skipped (e.g. when a prior step failed).
    Skipped,
}

impl StepStatus {
    /// Returns `true` if this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            StepStatus::Completed | StepStatus::Failed | StepStatus::Skipped
        )
    }
}

impl std::fmt::Display for StepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepStatus::Pending => f.write_str("Pending"),
            StepStatus::Running => f.write_str("Running"),
            StepStatus::Completed => f.write_str("Completed"),
            StepStatus::Failed => f.write_str("Failed"),
            StepStatus::Skipped => f.write_str("Skipped"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal() {
        assert!(StepStatus::Completed.is_terminal());
        assert!(StepStatus::Failed.is_terminal());
        assert!(StepStatus::Skipped.is_terminal());
        assert!(!StepStatus::Pending.is_terminal());
        assert!(!StepStatus::Running.is_terminal());
    }

    #[test]
    fn display() {
        assert_eq!(StepStatus::Pending.to_string(), "Pending");
        assert_eq!(StepStatus::Running.to_string(), "Running");
        assert_eq!(StepStatus::Completed.to_string(), "Completed");
        assert_eq!(StepStatus::Failed.to_string(), "Failed");
        assert_eq!(StepStatus::Skipped.to_string(), "Skipped");
    }
}
