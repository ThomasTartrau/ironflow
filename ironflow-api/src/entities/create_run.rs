//! Request type for triggering a workflow.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ironflow_store::models::MAX_IDEMPOTENCY_KEY_LEN;
use rust_decimal::Decimal;
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
///     max_retries: Some(2),
///     max_cost_usd: None,
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
    /// How many times the run may be replayed automatically after a transient
    /// failure. Defaults to `0`, meaning no automatic retry.
    ///
    /// Each retry waits an exponential backoff (30 s, 2 min, 8 min, capped at
    /// 15 min) before the run is replayed from the start. Failures that cannot
    /// succeed on replay -- an unknown workflow, an invalid payload, an
    /// exhausted agent budget, a rejected approval, a manual cancellation --
    /// consume no attempt.
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// Optional cumulative cost cap for this run, in USD.
    ///
    /// Overrides the workflow default and the server default. `None` falls back
    /// to those. Must be zero or positive.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<f64>))]
    #[serde(default)]
    pub max_cost_usd: Option<Decimal>,
}

impl CreateRunRequest {
    /// Validate the request body.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when `max_cost_usd` is negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::entities::CreateRunRequest;
    /// use rust_decimal::Decimal;
    ///
    /// let req = CreateRunRequest {
    ///     workflow: "deploy".to_string(),
    ///     payload: None,
    ///     labels: None,
    ///     scheduled_at: None,
    ///     max_retries: None,
    ///     max_cost_usd: Some(Decimal::new(-1, 0)),
    /// };
    /// assert!(req.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        match self.max_cost_usd {
            Some(cap) if cap < Decimal::ZERO => {
                Err("max_cost_usd must be zero or positive".to_string())
            }
            _ => Ok(()),
        }
    }
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

    fn request(max_cost_usd: Option<Decimal>) -> CreateRunRequest {
        CreateRunRequest {
            workflow: "deploy".to_string(),
            payload: None,
            labels: None,
            scheduled_at: None,
            max_retries: None,
            max_cost_usd,
        }
    }

    #[test]
    fn validate_accepts_absent_zero_and_positive_caps() {
        assert!(request(None).validate().is_ok());
        assert!(request(Some(Decimal::ZERO)).validate().is_ok());
        assert!(request(Some(Decimal::new(150, 2))).validate().is_ok());
    }

    #[test]
    fn validate_rejects_negative_cap() {
        let err = request(Some(Decimal::new(-1, 2)))
            .validate()
            .expect_err("negative cap must be rejected");
        assert!(err.contains("max_cost_usd"));
    }

    #[test]
    fn max_cost_usd_defaults_to_none_when_absent() {
        let req: CreateRunRequest =
            serde_json::from_str(r#"{"workflow":"deploy"}"#).expect("deserialize");
        assert!(req.max_cost_usd.is_none());
    }

    #[test]
    fn max_cost_usd_parses_from_json_number() {
        let req: CreateRunRequest =
            serde_json::from_str(r#"{"workflow":"deploy","max_cost_usd":2.5}"#)
                .expect("deserialize");
        assert_eq!(req.max_cost_usd, Some(Decimal::new(25, 1)));
    }

    #[test]
    fn max_retries_defaults_to_none_when_absent() {
        let req: CreateRunRequest =
            serde_json::from_str(r#"{"workflow":"deploy"}"#).expect("deserialize");
        assert!(req.max_retries.is_none());
    }

    #[test]
    fn max_retries_parses_from_json_number() {
        let req: CreateRunRequest =
            serde_json::from_str(r#"{"workflow":"deploy","max_retries":3}"#).expect("deserialize");
        assert_eq!(req.max_retries, Some(3));
    }

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
