//! Shared error conversion helpers.

use ironflow_core::error::OperationError;
use serde::Serialize;
use serde_json::Value;

/// Convert a [`kube::Error`] into an [`OperationError::Http`].
pub(crate) fn kube_err(e: kube::Error) -> OperationError {
    OperationError::Http {
        status: None,
        message: e.to_string(),
    }
}

/// Build an [`OperationError::External`] with `origin: "kubernetes"`.
///
/// Used by the high-level run-to-completion operations ([`PodRun`](crate::pod_run::PodRun),
/// [`JobRun`](crate::job_run::JobRun)) to surface infrastructure failures
/// (client init, create, wait, timeout) as external errors, distinct from
/// a command that ran but exited non-zero.
pub(crate) fn k8s_external(message: impl std::fmt::Display) -> OperationError {
    OperationError::External {
        origin: "kubernetes".to_string(),
        message: message.to_string(),
    }
}

/// Serialize a value into JSON, mapping errors to [`OperationError::Http`].
pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<Value, OperationError> {
    serde_json::to_value(value).map_err(|e| OperationError::Http {
        status: None,
        message: format!("serialization error: {e}"),
    })
}
