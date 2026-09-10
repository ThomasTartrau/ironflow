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

    /// All retry attempts exhausted.
    #[error("retries exhausted after {attempts} attempts: {source}")]
    Exhausted {
        /// Number of attempts made.
        attempts: u32,
        /// The last error encountered.
        source: Box<Error>,
    },
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

    /// Returns the HTTP status code if this is an API error (or an
    /// [`Exhausted`](Self::Exhausted) wrapping one).
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            Self::Exhausted { source, .. } => source.status(),
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

    /// Returns `true` if all retry attempts were exhausted.
    pub fn is_exhausted(&self) -> bool {
        matches!(self, Self::Exhausted { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_accessors() {
        let err = Error::api(404, "NOT_FOUND", "not found");
        assert!(err.is_api_error());
        assert!(!err.is_exhausted());
        assert_eq!(err.status(), Some(404));
        assert_eq!(err.code(), Some("NOT_FOUND"));
    }

    #[test]
    fn exhausted_delegates_status() {
        let inner = Error::api(503, "SERVICE_UNAVAILABLE", "unavailable");
        let err = Error::Exhausted {
            attempts: 3,
            source: Box::new(inner),
        };
        assert!(err.is_exhausted());
        assert!(!err.is_api_error());
        assert_eq!(err.status(), Some(503));
        assert_eq!(err.code(), None);
    }

    #[test]
    fn exhausted_with_non_api_source() {
        let inner = Error::Deserialize("bad json".to_string());
        let err = Error::Exhausted {
            attempts: 2,
            source: Box::new(inner),
        };
        assert!(err.is_exhausted());
        assert_eq!(err.status(), None);
    }

    #[test]
    fn deserialize_error() {
        let err = Error::Deserialize("bad json".to_string());
        assert!(!err.is_api_error());
        assert!(!err.is_exhausted());
        assert_eq!(err.status(), None);
        assert_eq!(err.code(), None);
    }

    #[test]
    fn sse_error() {
        let err = Error::Sse("connection reset".to_string());
        assert!(!err.is_api_error());
        assert!(!err.is_exhausted());
        assert_eq!(err.status(), None);
    }

    #[test]
    fn exhausted_display() {
        let inner = Error::api(503, "UNAVAILABLE", "down");
        let err = Error::Exhausted {
            attempts: 4,
            source: Box::new(inner),
        };
        let msg = format!("{err}");
        assert!(msg.contains("4 attempts"));
    }
}
