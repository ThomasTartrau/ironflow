//! Client-side rate limiter.
//!
//! Tracks the `Retry-After` header from 429 responses and delays
//! subsequent requests until the server-indicated window expires.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

/// Client-side rate limiter that respects `Retry-After` headers.
///
/// When a 429 response is received, the limiter records the expiration
/// instant. All subsequent requests through the client block
/// until that instant passes.
///
/// # Examples
///
/// ```
/// use ironflow_sdk::rate_limit::RateLimiter;
///
/// let limiter = RateLimiter::new();
/// assert!(!limiter.is_disabled());
/// ```
#[derive(Debug, Clone)]
pub struct RateLimiter {
    blocked_until: Arc<Mutex<Option<Instant>>>,
    enabled: bool,
}

impl RateLimiter {
    /// Create an enabled rate limiter.
    pub fn new() -> Self {
        Self {
            blocked_until: Arc::new(Mutex::new(None)),
            enabled: true,
        }
    }

    /// Create a disabled rate limiter (pass-through).
    pub fn disabled() -> Self {
        Self {
            blocked_until: Arc::new(Mutex::new(None)),
            enabled: false,
        }
    }

    /// Returns `true` if the rate limiter is disabled.
    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }

    /// Record a rate limit window from a `Retry-After` duration.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::rate_limit::RateLimiter;
    /// use std::time::Duration;
    ///
    /// # async fn example() {
    /// let limiter = RateLimiter::new();
    /// limiter.record(Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn record(&self, retry_after: Duration) {
        if !self.enabled {
            return;
        }
        let until = Instant::now() + retry_after;
        let mut guard = self.blocked_until.lock().await;
        *guard = Some(until);
    }

    /// Wait until the rate limit window expires (if any).
    pub(crate) async fn wait(&self) {
        if !self.enabled {
            return;
        }
        let deadline = {
            let guard = self.blocked_until.lock().await;
            *guard
        };
        if let Some(until) = deadline.filter(|&t| t > Instant::now()) {
            tokio::time::sleep_until(until).await;
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
