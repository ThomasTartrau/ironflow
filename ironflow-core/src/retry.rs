//! Retry policy for transient failures with exponential backoff.
//!
//! The [`RetryPolicy`] struct configures how operations (HTTP requests, agent
//! invocations) should be retried when they encounter transient errors. Retries
//! use exponential backoff with optional jitter to avoid thundering-herd effects.
//!
//! # Retryable errors
//!
//! Not all errors are retried. Only *transient* failures are eligible:
//!
//! | Operation | Retried | Not retried |
//! |-----------|---------|-------------|
//! | **HTTP** | Transport errors (DNS, timeout, connection refused), 5xx, 429 | SSRF blocks, 4xx (except 429), response too large |
//! | **Agent** | Process failures, timeouts, schema validation (CLI non-determinism) | Prompt too large |
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::operations::http::Http;
//! use ironflow_core::retry::RetryPolicy;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! // Simple: retry up to 3 times with default backoff
//! let output = Http::get("https://api.example.com/data")
//!     .retry(3)
//!     .await?;
//!
//! // Advanced: custom backoff settings
//! let output = Http::get("https://api.example.com/data")
//!     .retry_policy(
//!         RetryPolicy::new(3)
//!             .backoff(std::time::Duration::from_millis(500))
//!             .max_backoff(std::time::Duration::from_secs(30))
//!             .multiplier(3.0)
//!     )
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use crate::error::{AgentError, OperationError};

/// Default initial backoff between retries.
const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_millis(200);

/// Default maximum backoff between retries.
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Default backoff multiplier (doubles each retry).
const DEFAULT_MULTIPLIER: f64 = 2.0;

/// Retry policy with exponential backoff for transient failures.
///
/// Created via [`RetryPolicy::new`] with the maximum number of retries.
/// All other parameters have sensible defaults and can be customised with
/// builder methods.
///
/// # Defaults
///
/// | Parameter | Default |
/// |-----------|---------|
/// | `max_retries` | (required) |
/// | `initial_backoff` | 200ms |
/// | `max_backoff` | 30s |
/// | `multiplier` | 2.0 |
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use ironflow_core::retry::RetryPolicy;
///
/// let policy = RetryPolicy::new(3)
///     .backoff(Duration::from_millis(100))
///     .max_backoff(Duration::from_secs(10))
///     .multiplier(3.0);
/// ```
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub(crate) max_retries: u32,
    pub(crate) initial_backoff: Duration,
    pub(crate) max_backoff: Duration,
    pub(crate) multiplier: f64,
}

impl RetryPolicy {
    /// Create a new retry policy with the given maximum number of retries.
    ///
    /// The initial attempt is not counted as a retry, so `max_retries(3)` means
    /// up to 4 total attempts (1 initial + 3 retries).
    ///
    /// # Panics
    ///
    /// Panics if `max_retries` is `0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::retry::RetryPolicy;
    ///
    /// let policy = RetryPolicy::new(3);
    /// assert_eq!(policy.max_retries(), 3);
    /// ```
    pub fn new(max_retries: u32) -> Self {
        assert!(max_retries > 0, "max_retries must be greater than 0");
        Self {
            max_retries,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
            multiplier: DEFAULT_MULTIPLIER,
        }
    }

    /// Set the initial backoff duration before the first retry.
    ///
    /// Subsequent retries multiply this duration by the [`multiplier`](RetryPolicy::multiplier).
    ///
    /// # Panics
    ///
    /// Panics if `duration` is zero.
    pub fn backoff(mut self, duration: Duration) -> Self {
        assert!(!duration.is_zero(), "initial backoff must not be zero");
        self.initial_backoff = duration;
        self
    }

    /// Set the maximum backoff duration between retries.
    ///
    /// Even after many retries with exponential growth, the delay will never
    /// exceed this value.
    ///
    /// # Panics
    ///
    /// Panics if `duration` is zero.
    pub fn max_backoff(mut self, duration: Duration) -> Self {
        assert!(!duration.is_zero(), "max backoff must not be zero");
        self.max_backoff = duration;
        self
    }

    /// Set the backoff multiplier applied after each retry.
    ///
    /// For example, with `multiplier(2.0)` and `backoff(200ms)`:
    /// - Retry 1: 200ms
    /// - Retry 2: 400ms
    /// - Retry 3: 800ms
    ///
    /// # Panics
    ///
    /// Panics if `multiplier` is less than `1.0`, NaN, or infinity.
    pub fn multiplier(mut self, multiplier: f64) -> Self {
        assert!(
            multiplier >= 1.0 && multiplier.is_finite(),
            "multiplier must be >= 1.0 and finite, got {multiplier}"
        );
        self.multiplier = multiplier;
        self
    }

    /// Return the maximum number of retries.
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Compute the backoff duration for the given retry attempt (0-indexed).
    ///
    /// Returns the delay capped at [`max_backoff`](RetryPolicy::max_backoff).
    pub(crate) fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay = self.initial_backoff.as_secs_f64() * self.multiplier.powi(attempt as i32);
        let capped = delay.min(self.max_backoff.as_secs_f64());
        Duration::from_secs_f64(capped)
    }
}

/// Returns `true` if the given [`OperationError`] is transient and should be
/// retried.
///
/// # Retryable errors
///
/// - [`OperationError::Http`] with no status (transport error) or status 5xx / 429
/// - [`OperationError::Agent`] wrapping [`AgentError::ProcessFailed`] or [`AgentError::Timeout`]
/// - [`OperationError::Timeout`] (operation-level timeout)
///
/// # Non-retryable errors
///
/// - [`OperationError::Http`] with 4xx status (except 429)
/// - [`OperationError::Agent`] wrapping [`AgentError::PromptTooLarge`] or
///   [`AgentError::BudgetExceeded`] (the budget is already spent, replaying it
///   costs money and cannot succeed)
/// - [`OperationError::Shell`]
/// - [`OperationError::Deserialize`]
pub fn is_retryable(error: &OperationError) -> bool {
    match error {
        OperationError::Http { status, .. } => match status {
            None => true,
            Some(code) => *code >= 500 || *code == 429,
        },
        OperationError::Agent(agent_err) => match agent_err {
            AgentError::ProcessFailed { .. }
            | AgentError::Timeout { .. }
            | AgentError::SchemaValidation { .. }
            | AgentError::RateLimited { .. } => true,
            AgentError::HttpProvider { status_code, .. } => {
                *status_code == 0 || *status_code >= 500
            }
            AgentError::PromptTooLarge { .. } | AgentError::BudgetExceeded { .. } => false,
        },
        OperationError::Timeout { .. } => true,
        OperationError::Shell { .. } | OperationError::Deserialize { .. } => false,
    }
}

/// Returns `true` if the given HTTP status code indicates a transient server
/// error that should be retried (5xx or 429 Too Many Requests).
pub(crate) fn is_retryable_status(status: u16) -> bool {
    status >= 500 || status == 429
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // --- RetryPolicy builder ---

    #[test]
    fn new_creates_policy_with_defaults() {
        let policy = RetryPolicy::new(3);
        assert_eq!(policy.max_retries, 3);
        assert_eq!(policy.initial_backoff, DEFAULT_INITIAL_BACKOFF);
        assert_eq!(policy.max_backoff, DEFAULT_MAX_BACKOFF);
        assert!((policy.multiplier - DEFAULT_MULTIPLIER).abs() < f64::EPSILON);
    }

    #[test]
    #[should_panic(expected = "max_retries must be greater than 0")]
    fn new_zero_retries_panics() {
        let _ = RetryPolicy::new(0);
    }

    #[test]
    fn backoff_sets_initial_backoff() {
        let policy = RetryPolicy::new(1).backoff(Duration::from_secs(1));
        assert_eq!(policy.initial_backoff, Duration::from_secs(1));
    }

    #[test]
    #[should_panic(expected = "initial backoff must not be zero")]
    fn backoff_zero_panics() {
        let _ = RetryPolicy::new(1).backoff(Duration::ZERO);
    }

    #[test]
    fn max_backoff_sets_cap() {
        let policy = RetryPolicy::new(1).max_backoff(Duration::from_secs(60));
        assert_eq!(policy.max_backoff, Duration::from_secs(60));
    }

    #[test]
    #[should_panic(expected = "max backoff must not be zero")]
    fn max_backoff_zero_panics() {
        let _ = RetryPolicy::new(1).max_backoff(Duration::ZERO);
    }

    #[test]
    fn multiplier_sets_value() {
        let policy = RetryPolicy::new(1).multiplier(3.0);
        assert!((policy.multiplier - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    #[should_panic(expected = "multiplier must be >= 1.0")]
    fn multiplier_below_one_panics() {
        let _ = RetryPolicy::new(1).multiplier(0.5);
    }

    #[test]
    #[should_panic(expected = "multiplier must be >= 1.0 and finite")]
    fn multiplier_nan_panics() {
        let _ = RetryPolicy::new(1).multiplier(f64::NAN);
    }

    #[test]
    #[should_panic(expected = "multiplier must be >= 1.0 and finite")]
    fn multiplier_infinity_panics() {
        let _ = RetryPolicy::new(1).multiplier(f64::INFINITY);
    }

    #[test]
    fn max_retries_accessor() {
        assert_eq!(RetryPolicy::new(5).max_retries(), 5);
    }

    // --- delay_for_attempt ---

    #[test]
    fn delay_for_attempt_zero_is_initial_backoff() {
        let policy = RetryPolicy::new(3).backoff(Duration::from_millis(100));
        let delay = policy.delay_for_attempt(0);
        assert_eq!(delay, Duration::from_millis(100));
    }

    #[test]
    fn delay_for_attempt_grows_exponentially() {
        let policy = RetryPolicy::new(5)
            .backoff(Duration::from_millis(100))
            .multiplier(2.0);

        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(800));
    }

    #[test]
    fn delay_for_attempt_capped_at_max_backoff() {
        let policy = RetryPolicy::new(10)
            .backoff(Duration::from_secs(1))
            .max_backoff(Duration::from_secs(5))
            .multiplier(10.0);

        // attempt 0: 1s, attempt 1: 10s (capped to 5s)
        assert_eq!(policy.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_secs(5));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(5));
    }

    #[test]
    fn delay_for_attempt_with_multiplier_one_is_constant() {
        let policy = RetryPolicy::new(3)
            .backoff(Duration::from_millis(500))
            .multiplier(1.0);

        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(500));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(500));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(500));
    }

    // --- is_retryable ---

    #[test]
    fn http_transport_error_is_retryable() {
        let err = OperationError::Http {
            status: None,
            message: "connection refused".to_string(),
        };
        assert!(is_retryable(&err));
    }

    #[test]
    fn http_500_is_retryable() {
        let err = OperationError::Http {
            status: Some(500),
            message: "internal server error".to_string(),
        };
        assert!(is_retryable(&err));
    }

    #[test]
    fn http_502_is_retryable() {
        let err = OperationError::Http {
            status: Some(502),
            message: "bad gateway".to_string(),
        };
        assert!(is_retryable(&err));
    }

    #[test]
    fn http_503_is_retryable() {
        let err = OperationError::Http {
            status: Some(503),
            message: "service unavailable".to_string(),
        };
        assert!(is_retryable(&err));
    }

    #[test]
    fn http_429_is_retryable() {
        let err = OperationError::Http {
            status: Some(429),
            message: "too many requests".to_string(),
        };
        assert!(is_retryable(&err));
    }

    #[test]
    fn http_400_is_not_retryable() {
        let err = OperationError::Http {
            status: Some(400),
            message: "bad request".to_string(),
        };
        assert!(!is_retryable(&err));
    }

    #[test]
    fn http_404_is_not_retryable() {
        let err = OperationError::Http {
            status: Some(404),
            message: "not found".to_string(),
        };
        assert!(!is_retryable(&err));
    }

    #[test]
    fn agent_process_failed_is_retryable() {
        let err = OperationError::Agent(AgentError::ProcessFailed {
            exit_code: 1,
            stderr: "crash".to_string(),
        });
        assert!(is_retryable(&err));
    }

    #[test]
    fn agent_timeout_is_retryable() {
        let err = OperationError::Agent(AgentError::Timeout {
            limit: Duration::from_secs(60),
        });
        assert!(is_retryable(&err));
    }

    #[test]
    fn agent_prompt_too_large_is_not_retryable() {
        let err = OperationError::Agent(AgentError::PromptTooLarge {
            chars: 1_000_000,
            estimated_tokens: 250_000,
            model_limit: 200_000,
        });
        assert!(!is_retryable(&err));
    }

    #[test]
    fn agent_budget_exceeded_is_not_retryable() {
        let err = OperationError::Agent(AgentError::BudgetExceeded {
            spent_usd: 0.30,
            limit_usd: 0.25,
            debug_messages: Vec::new(),
            partial_usage: Box::default(),
        });
        assert!(!is_retryable(&err));
    }

    #[test]
    fn agent_schema_validation_is_retryable() {
        let err = OperationError::Agent(AgentError::SchemaValidation {
            expected: "object".to_string(),
            got: "string".to_string(),
            debug_messages: Vec::new(),
            partial_usage: Box::default(),
            raw_response: None,
        });
        assert!(is_retryable(&err));
    }

    #[test]
    fn operation_timeout_is_retryable() {
        let err = OperationError::Timeout {
            step: "fetch".to_string(),
            limit: Duration::from_secs(30),
        };
        assert!(is_retryable(&err));
    }

    #[test]
    fn shell_error_is_not_retryable() {
        let err = OperationError::Shell {
            exit_code: 1,
            stderr: "fail".to_string(),
        };
        assert!(!is_retryable(&err));
    }

    #[test]
    fn deserialize_error_is_not_retryable() {
        let err = OperationError::Deserialize {
            target_type: "MyStruct".to_string(),
            reason: "missing field".to_string(),
        };
        assert!(!is_retryable(&err));
    }

    // --- is_retryable_status ---

    #[test]
    fn retryable_status_codes() {
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(is_retryable_status(429));
    }

    #[test]
    fn non_retryable_status_codes() {
        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(201));
        assert!(!is_retryable_status(301));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(422));
        assert!(!is_retryable_status(428));
    }

    // --- builder chaining ---

    #[test]
    fn builder_chain_all_methods() {
        let policy = RetryPolicy::new(5)
            .backoff(Duration::from_millis(100))
            .max_backoff(Duration::from_secs(10))
            .multiplier(3.0);

        assert_eq!(policy.max_retries, 5);
        assert_eq!(policy.initial_backoff, Duration::from_millis(100));
        assert_eq!(policy.max_backoff, Duration::from_secs(10));
        assert!((policy.multiplier - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn clone_produces_independent_copy() {
        let policy = RetryPolicy::new(3).backoff(Duration::from_millis(100));
        let cloned = policy.clone();
        assert_eq!(policy.max_retries, cloned.max_retries);
        assert_eq!(policy.initial_backoff, cloned.initial_backoff);
    }

    #[test]
    fn debug_does_not_panic() {
        let policy = RetryPolicy::new(1);
        let debug = format!("{:?}", policy);
        assert!(debug.contains("RetryPolicy"));
    }
}
