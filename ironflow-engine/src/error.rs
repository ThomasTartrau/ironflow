//! Engine error types.

use thiserror::Error;

use ironflow_core::error::OperationError;
use ironflow_store::error::StoreError;

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
    fn serialization_error_from_conversion() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let engine_err = EngineError::from(serde_err);
        assert!(engine_err.to_string().contains("serialization error"));
    }
}
