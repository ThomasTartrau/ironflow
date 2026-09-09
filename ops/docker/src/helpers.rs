//! Internal helpers for error mapping and serialization.

use bollard::errors::Error;
use ironflow_core::error::OperationError;
use serde::Serialize;
use serde_json::Value;

pub(crate) fn docker_error(e: Error) -> OperationError {
    OperationError::External {
        origin: "docker".to_string(),
        message: e.to_string(),
    }
}

pub(crate) fn to_value<T: Serialize>(v: &T) -> Result<Value, OperationError> {
    serde_json::to_value(v).map_err(|e| OperationError::External {
        origin: "docker".to_string(),
        message: e.to_string(),
    })
}
