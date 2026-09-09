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

/// Serialize a value into JSON, mapping errors to [`OperationError::Http`].
pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<Value, OperationError> {
    serde_json::to_value(value).map_err(|e| OperationError::Http {
        status: None,
        message: format!("serialization error: {e}"),
    })
}
