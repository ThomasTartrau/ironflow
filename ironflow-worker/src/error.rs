//! Error types for the background worker.

use thiserror::Error;

use ironflow_core::error::OperationError;
use ironflow_engine::error::EngineError;
use ironflow_store::error::StoreError;

/// Errors produced by the worker.
///
/// # Examples
///
/// ```
/// use ironflow_worker::error::WorkerError;
///
/// let err = WorkerError::Shutdown("worker stopped".to_string());
/// assert!(err.to_string().contains("shutdown"));
/// ```
#[derive(Debug, Error)]
pub enum WorkerError {
    /// An operation (Shell, Http, Agent) failed during step execution.
    #[error("operation error: {0}")]
    Operation(#[from] OperationError),

    /// The engine returned an error.
    #[error("engine error: {0}")]
    Engine(#[from] EngineError),

    /// The store returned an error.
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// A shutdown-related error.
    #[error("shutdown error: {0}")]
    Shutdown(String),

    /// An internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_error_display() {
        let err = WorkerError::Shutdown("worker stopped".to_string());
        let msg = err.to_string();
        assert!(msg.contains("shutdown error"));
        assert!(msg.contains("worker stopped"));
    }

    #[test]
    fn internal_error_display() {
        let err = WorkerError::Internal("something went wrong".to_string());
        let msg = err.to_string();
        assert!(msg.contains("internal error"));
        assert!(msg.contains("something went wrong"));
    }

    #[test]
    fn shutdown_error_debug() {
        let err = WorkerError::Shutdown("test".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Shutdown"));
    }

    #[test]
    fn internal_error_debug() {
        let err = WorkerError::Internal("test".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Internal"));
    }

    #[test]
    fn operation_error_display() {
        use ironflow_core::error::OperationError;
        use std::time::Duration;

        let op_err = OperationError::Timeout {
            step: "shell".to_string(),
            limit: Duration::from_secs(30),
        };
        let err = WorkerError::Operation(op_err);
        let msg = err.to_string();
        assert!(msg.contains("operation error"));
    }

    #[test]
    fn operation_error_from_impl() {
        use ironflow_core::error::OperationError;
        use std::time::Duration;

        let op_err = OperationError::Timeout {
            step: "agent".to_string(),
            limit: Duration::from_secs(60),
        };
        let err: WorkerError = op_err.into();
        let msg = err.to_string();
        assert!(msg.contains("operation error"));
    }

    #[test]
    fn engine_error_display() {
        use ironflow_engine::error::EngineError;

        let engine_err = EngineError::InvalidWorkflow("unknown-handler".to_string());
        let err = WorkerError::Engine(engine_err);
        let msg = err.to_string();
        assert!(msg.contains("engine error"));
    }

    #[test]
    fn engine_error_from_impl() {
        use ironflow_engine::error::EngineError;

        let engine_err = EngineError::StepConfig("bad config".to_string());
        let err: WorkerError = engine_err.into();
        let msg = err.to_string();
        assert!(msg.contains("engine error"));
    }

    #[test]
    fn store_error_display() {
        use ironflow_store::error::StoreError;
        use uuid::Uuid;

        let store_err = StoreError::RunNotFound(Uuid::nil());
        let err = WorkerError::Store(store_err);
        let msg = err.to_string();
        assert!(msg.contains("store error"));
    }

    #[test]
    fn store_error_from_impl() {
        use ironflow_store::error::StoreError;
        use uuid::Uuid;

        let store_err = StoreError::StepNotFound(Uuid::nil());
        let err: WorkerError = store_err.into();
        let msg = err.to_string();
        assert!(msg.contains("store error"));
    }

    #[test]
    fn operation_error_debug() {
        use ironflow_core::error::OperationError;
        use std::time::Duration;

        let op_err = OperationError::Timeout {
            step: "test".to_string(),
            limit: Duration::from_secs(30),
        };
        let err = WorkerError::Operation(op_err);
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Operation"));
    }

    #[test]
    fn engine_error_debug() {
        use ironflow_engine::error::EngineError;

        let engine_err = EngineError::InvalidWorkflow("test".to_string());
        let err = WorkerError::Engine(engine_err);
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Engine"));
    }

    #[test]
    fn store_error_debug() {
        use ironflow_store::error::StoreError;
        use uuid::Uuid;

        let store_err = StoreError::RunNotFound(Uuid::nil());
        let err = WorkerError::Store(store_err);
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Store"));
    }
}
