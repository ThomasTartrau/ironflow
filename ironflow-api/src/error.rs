//! REST API error types and responses.
//!
//! [`ApiError`] is the primary error type for all API handlers. It implements
//! [`IntoResponse`] to serialize errors to JSON
//! with proper HTTP status codes.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ironflow_engine::error::MONTHLY_BUDGET_EXCEEDED_CODE;
use ironflow_store::error::StoreError;
use ironflow_types::ErrorEnvelope;
use serde_json::{Value, json};
use thiserror::Error;
use tracing::{error, warn};
use uuid::Uuid;

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

    /// The request conflicts with the current state of the resource (409).
    #[error("{0}")]
    Conflict(String),

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

    /// User not found (404).
    #[error("user not found")]
    UserNotFound(Uuid),

    /// Insufficient permissions for this action (403).
    #[error("insufficient permissions")]
    Forbidden,

    /// Secret not found (404).
    #[error("secret not found")]
    SecretNotFound(String),

    /// Insufficient scope (403).
    #[error("insufficient scope")]
    InsufficientScope,

    /// The idempotency key is already bound to a different request (409).
    ///
    /// Carries the run holding the key so the client can inspect it.
    #[error("idempotency key already used with a different request")]
    IdempotencyKeyConflict(Uuid),
    /// The global monthly cost quota is exhausted (429).
    ///
    /// Only blocks the creation of new runs; runs already in flight continue.
    #[error("{0}")]
    MonthlyBudgetExceeded(String),

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
            ApiError::Conflict(_) => "CONFLICT",
            ApiError::Unauthorized => "UNAUTHORIZED",
            ApiError::InvalidCredentials => "INVALID_CREDENTIALS",
            ApiError::DuplicateEmail => "DUPLICATE_EMAIL",
            ApiError::DuplicateUsername => "DUPLICATE_USERNAME",
            ApiError::ApiKeyNotFound(_) => "API_KEY_NOT_FOUND",
            ApiError::UserNotFound(_) => "USER_NOT_FOUND",
            ApiError::SecretNotFound(_) => "SECRET_NOT_FOUND",
            ApiError::Forbidden => "FORBIDDEN",
            ApiError::InsufficientScope => "INSUFFICIENT_SCOPE",
            ApiError::IdempotencyKeyConflict(_) => "IDEMPOTENCY_KEY_CONFLICT",
            ApiError::MonthlyBudgetExceeded(_) => MONTHLY_BUDGET_EXCEEDED_CODE,
            ApiError::Store(StoreError::Crypto(_)) => "SECRET_STORE_UNAVAILABLE",
            ApiError::Store(StoreError::LeaseLost { .. }) => "LEASE_LOST",
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
            ApiError::SecretNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            ApiError::DuplicateEmail => StatusCode::CONFLICT,
            ApiError::DuplicateUsername => StatusCode::CONFLICT,
            ApiError::ApiKeyNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::UserNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::InsufficientScope => StatusCode::FORBIDDEN,
            ApiError::IdempotencyKeyConflict(_) => StatusCode::CONFLICT,
            ApiError::MonthlyBudgetExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
            ApiError::Store(StoreError::Crypto(_)) => StatusCode::NOT_IMPLEMENTED,
            ApiError::Store(StoreError::LeaseLost { .. }) => StatusCode::CONFLICT,
            ApiError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Structured context attached to the JSON error body, if any.
    ///
    /// Never carries internal detail: only identifiers the caller is already
    /// entitled to see.
    fn details(&self) -> Option<Value> {
        match self {
            ApiError::IdempotencyKeyConflict(run_id) => Some(json!({ "run_id": run_id })),
            _ => None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code().to_string();
        let details = self.details();

        let message = match &self {
            ApiError::Store(StoreError::Crypto(_)) => {
                "secret store not configured (set IRONFLOW_SECRET_KEY)".to_string()
            }
            // Carries the caller-facing detail in its own Display impl,
            // unlike the generic "database error" of ApiError::Store.
            ApiError::Store(e @ StoreError::LeaseLost { .. }) => e.to_string(),
            _ => self.to_string(),
        };

        match &self {
            // A lost lease is a client-side condition, not a server fault:
            // it must not page anyone.
            ApiError::Store(e @ StoreError::LeaseLost { .. }) => {
                warn!(error = %e, code = %code, "lease refused")
            }
            ApiError::Store(e) => error!(error = %e, code = %code, "store error"),
            ApiError::Internal(detail) => {
                error!(detail = %detail, code = %code, "internal error")
            }
            _ => {}
        }

        let envelope = ErrorEnvelope {
            code,
            message,
            details,
        };

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
    fn conflict_status() {
        let err = ApiError::Conflict("run is already waiting for a retry".to_string());
        assert_eq!(err.status(), StatusCode::CONFLICT);
        assert_eq!(err.code(), "CONFLICT");
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

    #[test]
    fn user_not_found_status() {
        let err = ApiError::UserNotFound(Uuid::nil());
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        assert_eq!(err.code(), "USER_NOT_FOUND");
    }

    #[test]
    fn forbidden_status() {
        let err = ApiError::Forbidden;
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert_eq!(err.code(), "FORBIDDEN");
    }

    #[test]
    fn secret_not_found_status() {
        let err = ApiError::SecretNotFound("demo/api-key".to_string());
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        assert_eq!(err.code(), "SECRET_NOT_FOUND");
    }

    #[test]
    fn monthly_budget_exceeded_status_and_code() {
        let err = ApiError::MonthlyBudgetExceeded("quota exhausted".to_string());
        assert_eq!(err.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(err.code(), "MONTHLY_BUDGET_EXCEEDED");
        assert_eq!(err.to_string(), "quota exhausted");
    }
}
