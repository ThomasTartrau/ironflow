//! REST API error types and responses.
//!
//! [`ApiError`] is the primary error type for all API handlers. It implements
//! [`IntoResponse`] to serialize errors to JSON
//! with proper HTTP status codes.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ironflow_store::error::StoreError;
use serde::Serialize;
use serde_json::json;
use thiserror::Error;
use tracing::error;
use uuid::Uuid;

/// API error response envelope.
///
/// Serialized to JSON as: `{ "error": { "code": "...", "message": "..." } }`
#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    /// Machine-readable error code (e.g., "RUN_NOT_FOUND").
    pub code: String,
    /// Human-readable error message.
    pub message: String,
}

/// Error type for REST API operations.
///
/// Maps to appropriate HTTP status codes and error codes in the JSON response.
///
/// # Examples
///
/// ```
/// use ironflow_api::error::ApiError;
/// use uuid::Uuid;
///
/// let err = ApiError::RunNotFound(Uuid::nil());
/// assert_eq!(err.to_string(), "run not found");
/// ```
#[derive(Debug, Error)]
pub enum ApiError {
    /// The requested run does not exist (404).
    #[error("run not found")]
    RunNotFound(Uuid),

    /// The requested step does not exist (404).
    #[error("step not found")]
    StepNotFound(Uuid),

    /// Workflow not found (404).
    #[error("workflow not found")]
    WorkflowNotFound(String),

    /// Bad request: invalid input (400).
    #[error("{0}")]
    BadRequest(String),

    /// Authentication required (401).
    #[error("authentication required")]
    Unauthorized,

    /// Invalid credentials (401).
    #[error("invalid credentials")]
    InvalidCredentials,

    /// Email already taken (409).
    #[error("email already exists")]
    DuplicateEmail,

    /// Username already taken (409).
    #[error("username already exists")]
    DuplicateUsername,

    /// API key not found (404).
    #[error("API key not found")]
    ApiKeyNotFound(Uuid),

    /// Insufficient scope (403).
    #[error("insufficient scope")]
    InsufficientScope,

    /// Store operation failed (500).
    #[error("database error")]
    Store(#[from] StoreError),

    /// Internal server error (500).
    #[error("internal server error")]
    Internal(String),
}

impl ApiError {
    /// Return the error code for JSON serialization.
    fn code(&self) -> &str {
        match self {
            ApiError::RunNotFound(_) => "RUN_NOT_FOUND",
            ApiError::StepNotFound(_) => "STEP_NOT_FOUND",
            ApiError::WorkflowNotFound(_) => "WORKFLOW_NOT_FOUND",
            ApiError::BadRequest(_) => "BAD_REQUEST",
            ApiError::Unauthorized => "UNAUTHORIZED",
            ApiError::InvalidCredentials => "INVALID_CREDENTIALS",
            ApiError::DuplicateEmail => "DUPLICATE_EMAIL",
            ApiError::DuplicateUsername => "DUPLICATE_USERNAME",
            ApiError::ApiKeyNotFound(_) => "API_KEY_NOT_FOUND",
            ApiError::InsufficientScope => "INSUFFICIENT_SCOPE",
            ApiError::Store(_) => "DATABASE_ERROR",
            ApiError::Internal(_) => "INTERNAL_ERROR",
        }
    }

    /// Return the HTTP status code for this error.
    fn status(&self) -> StatusCode {
        match self {
            ApiError::RunNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::StepNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::WorkflowNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            ApiError::DuplicateEmail => StatusCode::CONFLICT,
            ApiError::DuplicateUsername => StatusCode::CONFLICT,
            ApiError::ApiKeyNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::InsufficientScope => StatusCode::FORBIDDEN,
            ApiError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code().to_string();
        let message = self.to_string();

        // Log internal details server-side before returning opaque message to client
        match &self {
            ApiError::Store(e) => error!(error = %e, code = %code, "store error"),
            ApiError::Internal(detail) => {
                error!(detail = %detail, code = %code, "internal error")
            }
            _ => {}
        }

        let envelope = ErrorEnvelope { code, message };

        (status, Json(json!({ "error": envelope }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_not_found_code() {
        let err = ApiError::RunNotFound(Uuid::nil());
        assert_eq!(err.code(), "RUN_NOT_FOUND");
    }

    #[test]
    fn run_not_found_status() {
        let err = ApiError::RunNotFound(Uuid::nil());
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn bad_request_status() {
        let err = ApiError::BadRequest("invalid field".to_string());
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert_eq!(err.code(), "BAD_REQUEST");
    }

    #[test]
    fn internal_error_status() {
        let err = ApiError::Internal("something went wrong".to_string());
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.code(), "INTERNAL_ERROR");
    }

    #[test]
    fn error_to_response() {
        let err = ApiError::BadRequest("invalid input".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn unauthorized_status() {
        let err = ApiError::Unauthorized;
        assert_eq!(err.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(err.code(), "UNAUTHORIZED");
    }

    #[test]
    fn invalid_credentials_status() {
        let err = ApiError::InvalidCredentials;
        assert_eq!(err.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(err.code(), "INVALID_CREDENTIALS");
    }

    #[test]
    fn duplicate_email_status() {
        let err = ApiError::DuplicateEmail;
        assert_eq!(err.status(), StatusCode::CONFLICT);
        assert_eq!(err.code(), "DUPLICATE_EMAIL");
    }

    #[test]
    fn duplicate_username_status() {
        let err = ApiError::DuplicateUsername;
        assert_eq!(err.status(), StatusCode::CONFLICT);
        assert_eq!(err.code(), "DUPLICATE_USERNAME");
    }

    #[test]
    fn workflow_not_found_status() {
        let err = ApiError::WorkflowNotFound("test".to_string());
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        assert_eq!(err.code(), "WORKFLOW_NOT_FOUND");
    }

    #[test]
    fn step_not_found_status() {
        let err = ApiError::StepNotFound(Uuid::nil());
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        assert_eq!(err.code(), "STEP_NOT_FOUND");
    }
}
