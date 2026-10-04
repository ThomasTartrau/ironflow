//! Request type for triggering a workflow.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ironflow_store::models::{MAX_CONCURRENCY_KEY_LEN, MAX_IDEMPOTENCY_KEY_LEN};
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
///     concurrency_key: Some("issue:12".to_string()),
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
    /// Optional exclusivity key, at most 255 bytes.
    ///
    /// While a non-terminal run (pending, running, sleeping, retrying,
    /// awaiting approval) holds the same key, the request is refused with
    /// `409 CONCURRENCY_CONFLICT` naming that run. The key is released when
    /// the run completes, fails, ends with a warning or is cancelled.
    #[serde(default)]
    pub concurrency_key: Option<String>,
}

impl CreateRunRequest {
    /// Validate the request body.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when `max_cost_usd` is negative, or
    /// when `concurrency_key` is blank or longer than
    /// [`MAX_CONCURRENCY_KEY_LEN`] bytes.
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
    ///     concurrency_key: None,
    /// };
    /// assert!(req.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if let Some(cap) = self.max_cost_usd
            && cap < Decimal::ZERO
        {
            return Err("max_cost_usd must be zero or positive".to_string());
        }
        match self.concurrency_key.as_deref() {
            Some(key) if key.trim().is_empty() => {
                Err("concurrency_key must not be empty".to_string())
            }
            Some(key) if key.len() > MAX_CONCURRENCY_KEY_LEN => Err(format!(
                "concurrency_key must be at most {MAX_CONCURRENCY_KEY_LEN} bytes"
            )),
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
    use serde_json::from_str;

    use super::*;

    fn request(max_cost_usd: Option<Decimal>) -> CreateRunRequest {
        CreateRunRequest {
            workflow: "deploy".to_string(),
            payload: None,
            labels: None,
            scheduled_at: None,
            max_retries: None,
            max_cost_usd,
            concurrency_key: None,
        }
    }

    fn keyed(concurrency_key: &str) -> CreateRunRequest {
        CreateRunRequest {
            concurrency_key: Some(concurrency_key.to_string()),
            ..request(None)
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
    fn concurrency_key_defaults_to_none_when_absent() {
        let req: CreateRunRequest = from_str(r#"{"workflow":"deploy"}"#).expect("deserialize");
        assert!(req.concurrency_key.is_none());
        assert!(req.validate().is_ok());
    }

    #[test]
    fn concurrency_key_parses_from_json() {
        let req: CreateRunRequest =
            from_str(r#"{"workflow":"deploy","concurrency_key":"issue:12"}"#).expect("deserialize");
        assert_eq!(req.concurrency_key.as_deref(), Some("issue:12"));
    }

    #[test]
    fn validate_accepts_a_concurrency_key_up_to_the_limit() {
        assert!(keyed("issue:12").validate().is_ok());
        assert!(keyed("cl\u{e9}:\u{e9}lodie").validate().is_ok());
        let longest = "k".repeat(MAX_CONCURRENCY_KEY_LEN);
        assert!(keyed(&longest).validate().is_ok());
    }

    #[test]
    fn validate_rejects_an_empty_or_blank_concurrency_key() {
        for key in ["", "   ", "\t\n"] {
            let err = keyed(key)
                .validate()
                .expect_err("blank key must be rejected");
            assert_eq!(err, "concurrency_key must not be empty");
        }
    }

    #[test]
    fn validate_rejects_a_concurrency_key_over_the_limit() {
        let err = keyed(&"k".repeat(MAX_CONCURRENCY_KEY_LEN + 1))
            .validate()
            .expect_err("over-long key must be rejected");
        assert_eq!(err, "concurrency_key must be at most 255 bytes");
    }

    #[test]
    fn validate_counts_the_concurrency_key_limit_in_bytes() {
        // 128 two-byte characters: 128 chars but 256 bytes.
        let err = keyed(&"\u{e9}".repeat(128))
            .validate()
            .expect_err("the limit is in bytes, not characters");
        assert!(err.contains("255 bytes"));
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
