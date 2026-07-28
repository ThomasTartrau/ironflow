//! Engine error types.

use rust_decimal::Decimal;
use thiserror::Error;

use ironflow_core::error::OperationError;
use ironflow_store::error::StoreError;

/// Business error code carried by [`EngineError::RunBudgetExceeded`].
pub const RUN_BUDGET_EXCEEDED_CODE: &str = "RUN_BUDGET_EXCEEDED";

/// Business error code carried by [`EngineError::MonthlyBudgetExceeded`].
pub const MONTHLY_BUDGET_EXCEEDED_CODE: &str = "MONTHLY_BUDGET_EXCEEDED";

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
    fn serialization_error_from_conversion() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let engine_err = EngineError::from(serde_err);
        assert!(engine_err.to_string().contains("serialization error"));
    }
}
