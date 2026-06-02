//! SDK error types.

use ironflow_types::ErrorEnvelope;
use serde::Deserialize;
use thiserror::Error;

/// Wrapper for the API error envelope.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ApiErrorEnvelope {
    pub error: ErrorEnvelope,
}

/// SDK error type.
///
/// Distinguishes between network errors, API errors (with status code and
/// structured body), and deserialization errors.
///
/// # Examples
///
/// ```
/// use ironflow_sdk::Error;
///
/// let err = Error::api(404, "RUN_NOT_FOUND", "run not found");
/// assert!(err.is_api_error());
/// ```
#[derive(Debug, Error)]
pub enum Error {
    /// HTTP transport error (network unreachable, DNS, TLS, timeout).
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// The API returned an error response with a structured body.
    #[error("API error {status}: [{code}] {message}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Machine-readable error code.
        code: String,
        /// Human-readable error message.
        message: String,
    },

    /// Failed to deserialize the response body.
    #[error("deserialization error: {0}")]
    Deserialize(String),

    /// SSE stream error.
    #[error("SSE error: {0}")]
    Sse(String),
}

impl Error {
    /// Create an API error from its components.
    pub fn api(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Api {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    /// Returns `true` if this is an API error (not a transport error).
    pub fn is_api_error(&self) -> bool {
        matches!(self, Self::Api { .. })
    }

    /// Returns the HTTP status code if this is an API error.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Returns the API error code if this is an API error.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api { code, .. } => Some(code),
            _ => None,
        }
    }
}
