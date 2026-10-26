//! Conversion of a GitLab API error into an [`OperationError`].

use gitlab::RestError;
use gitlab::api::ApiError;
use ironflow_core::error::OperationError;

/// Convert the error of a GitLab query into [`OperationError::Http`].
///
/// `message` is the full `Display` of the error, so the text GitLab answered
/// stays visible. `status` is the HTTP status of the answer when there was one.
pub(crate) fn http_error(err: ApiError<RestError>) -> OperationError {
    // Kept as a typed field so callers test `Some(409)` instead of parsing the
    // message.
    let status = match &err {
        ApiError::GitlabService { status, .. }
        | ApiError::GitlabWithStatus { status, .. }
        | ApiError::GitlabUnrecognizedWithStatus { status, .. } => Some(status.as_u16()),
        // No HTTP answer was received, or its status was not kept.
        _ => None,
    };
    OperationError::Http {
        status,
        message: err.to_string(),
    }
}
