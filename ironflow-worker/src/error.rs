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
}
