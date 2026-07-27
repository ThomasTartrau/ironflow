//! Request type for triggering a workflow.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ironflow_store::models::MAX_IDEMPOTENCY_KEY_LEN;
use serde::Deserialize;
use serde_json::Value;

/// Request to trigger a workflow.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::CreateRunRequest;
/// use serde_json::json;
///
/// let req = CreateRunRequest {
///     workflow: "deploy".to_string(),
///     payload: Some(json!({"env": "prod"})),
///     labels: None,
///     scheduled_at: None,
/// };
/// assert_eq!(req.workflow, "deploy");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct CreateRunRequest {
    /// The workflow name to trigger.
    pub workflow: String,
    /// Optional input payload for the workflow.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<std::collections::HashMap<String, serde_json::Value>>))]
    pub payload: Option<Value>,
    /// Optional key-value labels for categorization and filtering.
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
    /// Optional deferred execution time. `None` means run immediately.
    #[serde(default)]
    pub scheduled_at: Option<DateTime<Utc>>,
}

/// Why an `Idempotency-Key` header value was rejected.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::{IdempotencyKeyError, validate_idempotency_key};
///
/// assert_eq!(
///     validate_idempotency_key(""),
///     Err(IdempotencyKeyError::Empty),
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdempotencyKeyError {
    /// The header was present but carried no value.
    Empty,
    /// The value exceeds [`MAX_IDEMPOTENCY_KEY_LEN`] bytes.
    TooLong,
    /// The value contains a byte outside printable ASCII.
    NotPrintableAscii,
}

impl IdempotencyKeyError {
    /// Client-facing explanation of the rejection.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::entities::IdempotencyKeyError;
    ///
    /// assert!(IdempotencyKeyError::Empty.message().contains("empty"));
    /// ```
    pub fn message(&self) -> String {
        match self {
            IdempotencyKeyError::Empty => "Idempotency-Key must not be empty".to_string(),
            IdempotencyKeyError::TooLong => {
                format!("Idempotency-Key must be at most {MAX_IDEMPOTENCY_KEY_LEN} bytes")
            }
            IdempotencyKeyError::NotPrintableAscii => {
                "Idempotency-Key must contain only printable ASCII characters".to_string()
            }
        }
    }
}

/// Validate an `Idempotency-Key` header value.
///
/// A key must be non-empty, at most [`MAX_IDEMPOTENCY_KEY_LEN`] bytes, and made
/// only of printable ASCII. Empty keys are rejected because they would otherwise
/// become a single key shared by every client.
///
/// # Errors
///
/// Returns [`IdempotencyKeyError`] describing which rule the value broke.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::{IdempotencyKeyError, validate_idempotency_key};
///
/// assert!(validate_idempotency_key("github:abc-123").is_ok());
/// assert_eq!(
///     validate_idempotency_key("clé"),
///     Err(IdempotencyKeyError::NotPrintableAscii),
/// );
/// ```
pub fn validate_idempotency_key(key: &str) -> Result<(), IdempotencyKeyError> {
    if key.is_empty() {
        return Err(IdempotencyKeyError::Empty);
    }
    if key.len() > MAX_IDEMPOTENCY_KEY_LEN {
        return Err(IdempotencyKeyError::TooLong);
    }
    if !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(IdempotencyKeyError::NotPrintableAscii);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_provider_delivery_id() {
        assert!(validate_idempotency_key("github:8f4e2a10-1234-4bcd-9876-abcdef012345").is_ok());
    }

    #[test]
    fn rejects_an_empty_key() {
        assert_eq!(
            validate_idempotency_key(""),
            Err(IdempotencyKeyError::Empty)
        );
    }

    #[test]
    fn accepts_a_key_at_the_length_limit() {
        let key = "a".repeat(MAX_IDEMPOTENCY_KEY_LEN);
        assert!(validate_idempotency_key(&key).is_ok());
    }

    #[test]
    fn rejects_a_key_one_byte_over_the_limit() {
        let key = "a".repeat(MAX_IDEMPOTENCY_KEY_LEN + 1);
        assert_eq!(
            validate_idempotency_key(&key),
            Err(IdempotencyKeyError::TooLong)
        );
    }

    #[test]
    fn rejects_non_ascii() {
        assert_eq!(
            validate_idempotency_key("clé-🚀"),
            Err(IdempotencyKeyError::NotPrintableAscii)
        );
    }

    #[test]
    fn rejects_control_characters() {
        assert_eq!(
            validate_idempotency_key("abc\ndef"),
            Err(IdempotencyKeyError::NotPrintableAscii)
        );
    }

    #[test]
    fn rejects_a_space() {
        // `is_ascii_graphic` excludes the space: a bare space is not a usable key.
        assert_eq!(
            validate_idempotency_key("abc def"),
            Err(IdempotencyKeyError::NotPrintableAscii)
        );
    }

    #[test]
    fn error_messages_name_the_broken_rule() {
        assert!(IdempotencyKeyError::Empty.message().contains("empty"));
        assert!(
            IdempotencyKeyError::TooLong
                .message()
                .contains(&MAX_IDEMPOTENCY_KEY_LEN.to_string())
        );
        assert!(
            IdempotencyKeyError::NotPrintableAscii
                .message()
                .contains("ASCII")
        );
    }
}
