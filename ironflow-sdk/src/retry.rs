//! Automatic retry with exponential backoff and jitter.
//!
//! Retries transient HTTP errors (429, 502, 503, 504) and connection/timeout
//! failures. Respects the `Retry-After` header when present.

use std::time::Duration;

use rand::Rng;
use reqwest::{Error as ReqwestError, Response, StatusCode};

/// Configuration for automatic request retries.
///
/// # Examples
///
/// ```
/// use ironflow_sdk::retry::RetryConfig;
/// use std::time::Duration;
///
/// let config = RetryConfig {
///     max_retries: 3,
///     base_delay: Duration::from_millis(500),
///     max_delay: Duration::from_secs(30),
/// };
/// assert_eq!(config.max_retries, 3);
/// ```
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries).
    pub max_retries: u32,
    /// Base delay for exponential backoff (doubled each attempt).
    pub base_delay: Duration,
    /// Maximum delay cap.
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Returns `true` if the status code is retryable (429, 502, 503, 504).
pub(crate) fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

/// Returns `true` if the reqwest error is a transient network/timeout failure.
pub(crate) fn is_retryable_error(err: &ReqwestError) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

/// Compute the backoff delay for a given attempt, with jitter.
///
/// Formula: `min(base * 2^attempt + jitter, max_delay)` where jitter
/// is a random value in `[0, 50%]` of the computed delay.
pub(crate) fn backoff_delay(config: &RetryConfig, attempt: u32) -> Duration {
    let base_ms = config.base_delay.as_millis() as u64;
    let exp_ms = base_ms.saturating_mul(1u64 << attempt.min(63));
    let max_ms = config.max_delay.as_millis() as u64;
    let capped_ms = exp_ms.min(max_ms);

    let jitter_ms = rand::rng().random_range(0..=capped_ms / 2);
    let total_ms = capped_ms.saturating_add(jitter_ms).min(max_ms);

    Duration::from_millis(total_ms)
}

/// Parse the `Retry-After` header value as seconds.
///
/// Returns `None` if the header is missing or not a valid integer.
pub(crate) fn parse_retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}
