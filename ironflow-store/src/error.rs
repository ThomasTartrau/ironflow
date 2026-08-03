//! Storage error types.
//!
//! [`StoreError`] covers all failure modes for [`RunStore`](crate::store::RunStore)
//! implementations, from missing records to invalid FSM transitions.

use thiserror::Error;
use uuid::Uuid;

use crate::entities::RunStatus;

/// Errors produced by [`RunStore`](crate::store::RunStore) operations.
///
/// # Examples
///
/// ```
/// use ironflow_store::error::StoreError;
/// use uuid::Uuid;
///
/// let err = StoreError::RunNotFound(Uuid::nil());
/// assert!(err.to_string().contains("run not found"));
/// ```
#[derive(Debug, Error)]
pub enum StoreError {
    /// The requested run does not exist.
    #[error("run not found: {0}")]
    RunNotFound(Uuid),

    /// The requested step does not exist.
    #[error("step not found: {0}")]
    StepNotFound(Uuid),

    /// Attempted an illegal FSM transition.
    #[error("invalid status transition: {from} -> {to}")]
    InvalidTransition {
        /// Current status.
        from: RunStatus,
        /// Attempted target status.
        to: RunStatus,
    },

    /// Email is already taken by another user.
    #[error("email already exists: {0}")]
    DuplicateEmail(String),

    /// Username is already taken by another user.
    #[error("username already exists: {0}")]
    DuplicateUsername(String),

    /// The requested user does not exist.
    #[error("user not found: {0}")]
    UserNotFound(Uuid),

    /// The worker no longer holds the lease on this run.
    ///
    /// Either another worker took it over after the lease expired, or the run
    /// left the `Running` state (cancelled, failed, awaiting approval).
    #[error("lease lost on run {run_id}")]
    LeaseLost {
        /// Run whose lease was lost.
        run_id: Uuid,
        /// Worker currently holding the lease, if any.
        held_by: Option<String>,
    },

    /// The step already holds an artifact with this name.
    #[error("artifact {name:?} already exists on step {step_id}")]
    DuplicateArtifact {
        /// Step that already owns the name.
        step_id: Uuid,
        /// The conflicting artifact name.
        name: String,
    },

    /// A database or I/O error from the backing store.
    #[error("database error: {0}")]
    Database(String),

    /// JSON serialization or deserialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// A cryptographic operation failed (encryption or decryption).
    #[error("crypto error: {0}")]
    Crypto(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_not_found_display() {
        let id = Uuid::nil();
        let err = StoreError::RunNotFound(id);
        assert_eq!(err.to_string(), format!("run not found: {id}"));
    }

    #[test]
    fn step_not_found_display() {
        let id = Uuid::nil();
        let err = StoreError::StepNotFound(id);
        assert_eq!(err.to_string(), format!("step not found: {id}"));
    }

    #[test]
    fn invalid_transition_display() {
        let err = StoreError::InvalidTransition {
            from: RunStatus::Pending,
            to: RunStatus::Completed,
        };
        assert!(err.to_string().contains("Pending"));
        assert!(err.to_string().contains("Completed"));
    }

    #[test]
    fn database_error_display() {
        let err = StoreError::Database("connection refused".to_string());
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn serialization_error_from_serde() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let err = StoreError::from(serde_err);
        assert!(err.to_string().contains("serialization error"));
    }
}
