//! Engine error types.

use rust_decimal::Decimal;
use thiserror::Error;

use ironflow_artifacts::error::ArtifactError;
use ironflow_core::error::OperationError;
use ironflow_store::error::StoreError;

use crate::guard::{WORKFLOW_GUARD_REJECTED_CODE, WorkflowRejection};

/// Business error code carried by [`EngineError::RunBudgetExceeded`].
pub const RUN_BUDGET_EXCEEDED_CODE: &str = "RUN_BUDGET_EXCEEDED";

/// Business error code carried by [`EngineError::MonthlyBudgetExceeded`].
pub const MONTHLY_BUDGET_EXCEEDED_CODE: &str = "MONTHLY_BUDGET_EXCEEDED";

/// Business error code for handler-version mismatch on retry.
pub const HANDLER_VERSION_MISMATCH_CODE: &str = "HANDLER_VERSION_MISMATCH";

/// Errors produced by the workflow engine.
#[derive(Debug, Error)]
pub enum EngineError {
    /// An operation (Shell, Http, Agent) failed during step execution.
    #[error("operation failed: {0}")]
    Operation(#[from] OperationError),

    /// The backing store returned an error.
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// The workflow definition is invalid.
    #[error("invalid workflow: {0}")]
    InvalidWorkflow(String),

    /// A step configuration could not be deserialized for execution.
    #[error("step config error: {0}")]
    StepConfig(String),

    /// JSON serialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The run reached its cumulative cost cap before launching an agent step.
    ///
    /// Raised *before* the step is created, so no work and no spend happen.
    /// The engine transitions the run to
    /// [`Cancelled`](ironflow_store::entities::RunStatus::Cancelled).
    #[error(
        "{RUN_BUDGET_EXCEEDED_CODE}: run {run_id} would exceed its cost cap \
         (spent {spent_usd} USD + next step {step_budget_usd} USD > cap {limit_usd} USD)"
    )]
    RunBudgetExceeded {
        /// The run that hit its cap.
        run_id: uuid::Uuid,
        /// The configured cap, in USD.
        limit_usd: Decimal,
        /// Cost already accumulated by this run and its ancestors, in USD.
        spent_usd: Decimal,
        /// Declared budget of the step that was about to run, in USD.
        step_budget_usd: Decimal,
    },

    /// The global monthly cost quota is exhausted; no new run may be created.
    ///
    /// Runs already in flight are never interrupted by this error.
    #[error(
        "{MONTHLY_BUDGET_EXCEEDED_CODE}: monthly cost quota exhausted \
         ({spent_usd} USD spent of {limit_usd} USD)"
    )]
    MonthlyBudgetExceeded {
        /// The configured monthly quota, in USD.
        limit_usd: Decimal,
        /// Cost already spent during the current calendar month, in USD.
        spent_usd: Decimal,
    },

    /// A step declared an output that produced no file.
    ///
    /// Raised only when the step itself succeeded: a declared output that never
    /// materialised is a broken contract, and failing here beats failing later
    /// in whichever step tried to consume it.
    #[error("step {step:?} declared output {pattern:?} but no file matched")]
    MissingArtifact {
        /// Name of the step that declared the output.
        step: String,
        /// The unmatched pattern.
        pattern: String,
    },

    /// A step asked for an artifact that no earlier step produced.
    #[error("no artifact {name:?} produced by step {step:?} before this point")]
    ArtifactNotFound {
        /// Name of the producing step that was searched for.
        step: String,
        /// Name of the artifact that was searched for.
        name: String,
    },

    /// Artifacts were used but no storage backend is configured.
    #[error("artifact storage is not configured: {0}")]
    ArtifactsUnavailable(String),

    /// The artifact storage backend failed.
    #[error("artifact storage error: {0}")]
    Artifact(#[from] ArtifactError),

    /// The run requires human approval before continuing.
    #[error("approval required for run {run_id}, step {step_id}: {message}")]
    ApprovalRequired {
        /// The run that is awaiting approval.
        run_id: uuid::Uuid,
        /// The approval step that triggered the pause.
        step_id: uuid::Uuid,
        /// The approval message.
        message: String,
    },

    /// A workflow invocation was rejected by the [workflow guard](crate::guard).
    ///
    /// The run is transitioned to
    /// [`Cancelled`](ironflow_store::entities::RunStatus::Cancelled) when this
    /// error is raised.
    #[error("{WORKFLOW_GUARD_REJECTED_CODE}: {0}")]
    WorkflowGuardRejected(#[from] WorkflowRejection),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_workflow_display() {
        let err = EngineError::InvalidWorkflow("unknown-handler".to_string());
        assert!(err.to_string().contains("invalid workflow"));
        assert!(err.to_string().contains("unknown-handler"));
    }

    #[test]
    fn step_config_display() {
        let err = EngineError::StepConfig("bad shell config".to_string());
        assert!(err.to_string().contains("step config error"));
        assert!(err.to_string().contains("bad shell config"));
    }

    #[test]
    fn store_error_from_conversion() {
        let store_err = StoreError::RunNotFound(uuid::Uuid::nil());
        let engine_err = EngineError::from(store_err);
        assert!(engine_err.to_string().contains("store error"));
    }

    #[test]
    fn run_budget_exceeded_display_carries_code_and_amounts() {
        let err = EngineError::RunBudgetExceeded {
            run_id: uuid::Uuid::nil(),
            limit_usd: Decimal::new(200, 2),
            spent_usd: Decimal::new(180, 2),
            step_budget_usd: Decimal::new(50, 2),
        };

        let msg = err.to_string();
        assert!(msg.contains(RUN_BUDGET_EXCEEDED_CODE));
        assert!(msg.contains("2.00"));
        assert!(msg.contains("1.80"));
        assert!(msg.contains("0.50"));
    }

    #[test]
    fn monthly_budget_exceeded_display_carries_code_and_amounts() {
        let err = EngineError::MonthlyBudgetExceeded {
            limit_usd: Decimal::new(10000, 2),
            spent_usd: Decimal::new(10500, 2),
        };

        let msg = err.to_string();
        assert!(msg.contains(MONTHLY_BUDGET_EXCEEDED_CODE));
        assert!(msg.contains("100.00"));
        assert!(msg.contains("105.00"));
    }

    #[test]
    fn missing_artifact_display_names_the_step_and_pattern() {
        let err = EngineError::MissingArtifact {
            step: "build".to_string(),
            pattern: "target/report.html".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("\"build\""));
        assert!(msg.contains("target/report.html"));
    }

    #[test]
    fn artifact_not_found_display_names_the_producer() {
        let err = EngineError::ArtifactNotFound {
            step: "build".to_string(),
            name: "report.html".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("\"build\""));
        assert!(msg.contains("report.html"));
    }

    #[test]
    fn artifacts_unavailable_display() {
        let err = EngineError::ArtifactsUnavailable("no blob store".to_string());
        assert!(err.to_string().contains("not configured"));
    }

    #[test]
    fn artifact_error_from_conversion() {
        let engine_err = EngineError::from(ArtifactError::NotFound("a/b".to_string()));
        assert!(engine_err.to_string().contains("artifact storage error"));
    }

    #[test]
    fn serialization_error_from_conversion() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let engine_err = EngineError::from(serde_err);
        assert!(engine_err.to_string().contains("serialization error"));
    }

    #[test]
    fn workflow_guard_rejected_display_carries_code_and_detail() {
        use crate::guard::WorkflowRejection;

        let rejection = WorkflowRejection::MaxDepthExceeded { depth: 6, max: 5 };
        let err = EngineError::from(rejection);

        let msg = err.to_string();
        assert!(msg.contains(WORKFLOW_GUARD_REJECTED_CODE));
        assert!(msg.contains("max call depth exceeded"));
        assert!(msg.contains("6/5"));
    }

    #[test]
    fn workflow_guard_rejected_from_conversion() {
        use crate::guard::WorkflowRejection;

        let rejection = WorkflowRejection::CycleDetected {
            target: "wf-b".to_string(),
            chain: vec!["wf-a".to_string(), "wf-b".to_string()],
        };
        let engine_err = EngineError::from(rejection);
        assert!(engine_err.to_string().contains("cycle detected"));
    }
}
