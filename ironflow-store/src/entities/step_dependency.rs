//! [`StepDependency`] entity -- DAG edges between steps.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A dependency edge in the step DAG: `step_id` depends on `depends_on`.
///
/// Created automatically when steps execute after a parallel batch or
/// after a preceding sequential step.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::StepDependency;
/// use uuid::Uuid;
/// use chrono::Utc;
///
/// let dep = StepDependency {
///     step_id: Uuid::nil(),
///     depends_on: Uuid::nil(),
///     created_at: Utc::now(),
/// };
/// assert_eq!(dep.step_id, dep.depends_on);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepDependency {
    /// The step that has a dependency.
    pub step_id: Uuid,
    /// The step that must complete first.
    pub depends_on: Uuid,
    /// When this dependency was recorded.
    pub created_at: DateTime<Utc>,
}

/// Request to create a new dependency edge.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::NewStepDependency;
/// use uuid::Uuid;
///
/// let dep = NewStepDependency {
///     step_id: Uuid::nil(),
///     depends_on: Uuid::nil(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewStepDependency {
    /// The step that has a dependency.
    pub step_id: Uuid,
    /// The step that must complete first.
    pub depends_on: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_dependency_serde_roundtrip() {
        let dep = StepDependency {
            step_id: Uuid::nil(),
            depends_on: Uuid::nil(),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&dep).expect("serialize");
        let back: StepDependency = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.step_id, dep.step_id);
        assert_eq!(back.depends_on, dep.depends_on);
    }

    #[test]
    fn new_step_dependency_serde_roundtrip() {
        let dep = NewStepDependency {
            step_id: Uuid::nil(),
            depends_on: Uuid::nil(),
        };
        let json = serde_json::to_string(&dep).expect("serialize");
        let back: NewStepDependency = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.step_id, dep.step_id);
    }
}
