//! Caps on simultaneous Server-Sent Events connections.
//!
//! An SSE stream (`GET /api/v1/events`, `GET /api/v1/runs/{id}/events`) holds
//! a connection, a broadcast receiver and a task for as long as the client
//! keeps it open. [`SseLimiter`] bounds how many of them one principal (a user,
//! or the owner of an API key) and the whole server may hold at once. A
//! connection beyond either cap is refused with `429 SSE_CONNECTION_LIMIT`.
//!
//! # Examples
//!
//! ```
//! use ironflow_api::sse_limiter::{SseLimiter, SseLimits};
//! use uuid::Uuid;
//!
//! # fn example() -> Result<(), ironflow_api::error::ApiError> {
//! let limiter = SseLimiter::new(SseLimits {
//!     per_principal: Some(1),
//!     global: None,
//! });
//! let user = Uuid::now_v7();
//!
//! let connection = limiter.try_acquire(user)?;
//! assert!(limiter.try_acquire(user).is_err());
//! drop(connection);
//! assert!(limiter.try_acquire(user).is_ok());
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use uuid::Uuid;

use crate::error::ApiError;

/// Default cap on simultaneous SSE streams per principal.
pub const DEFAULT_SSE_MAX_PER_PRINCIPAL: usize = 5;

/// Default cap on simultaneous SSE streams for the whole server.
pub const DEFAULT_SSE_MAX_GLOBAL: usize = 1000;

/// Caps applied by an [`SseLimiter`]. `None` means unlimited.
///
/// # Examples
///
/// ```
/// use ironflow_api::sse_limiter::{
///     DEFAULT_SSE_MAX_GLOBAL, DEFAULT_SSE_MAX_PER_PRINCIPAL, SseLimits,
/// };
///
/// let limits = SseLimits::default();
/// assert_eq!(limits.per_principal, Some(DEFAULT_SSE_MAX_PER_PRINCIPAL));
/// assert_eq!(limits.global, Some(DEFAULT_SSE_MAX_GLOBAL));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SseLimits {
    /// Simultaneous streams one principal may hold.
    pub per_principal: Option<usize>,
    /// Simultaneous streams the whole server may hold.
    pub global: Option<usize>,
}

impl Default for SseLimits {
    fn default() -> Self {
        Self {
            per_principal: Some(DEFAULT_SSE_MAX_PER_PRINCIPAL),
            global: Some(DEFAULT_SSE_MAX_GLOBAL),
        }
    }
}

/// Open stream counters, shared by every clone of an [`SseLimiter`].
#[derive(Debug, Default)]
struct Counts {
    total: usize,
    per_principal: HashMap<Uuid, usize>,
}

/// Counts open SSE streams and refuses new ones past the [`SseLimits`].
///
/// Cheap to clone: clones share the same counters.
///
/// # Examples
///
/// ```
/// use ironflow_api::sse_limiter::SseLimiter;
/// use uuid::Uuid;
///
/// # fn example() -> Result<(), ironflow_api::error::ApiError> {
/// let limiter = SseLimiter::default();
/// let _connection = limiter.try_acquire(Uuid::now_v7())?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct SseLimiter {
    limits: SseLimits,
    counts: Arc<Mutex<Counts>>,
}

impl SseLimiter {
    /// Create a limiter enforcing `limits`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::sse_limiter::{SseLimiter, SseLimits};
    ///
    /// let limiter = SseLimiter::new(SseLimits {
    ///     per_principal: Some(2),
    ///     global: Some(100),
    /// });
    /// ```
    pub fn new(limits: SseLimits) -> Self {
        Self {
            limits,
            counts: Arc::new(Mutex::new(Counts::default())),
        }
    }

    /// Reserve a stream slot for `principal`.
    ///
    /// The slot is held by the returned [`SseConnection`] and released when
    /// it is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::SseConnectionLimit`] when the server or the
    /// principal already holds as many streams as its limit allows.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::sse_limiter::{SseLimiter, SseLimits};
    /// use uuid::Uuid;
    ///
    /// # fn example() -> Result<(), ironflow_api::error::ApiError> {
    /// let limiter = SseLimiter::new(SseLimits {
    ///     per_principal: None,
    ///     global: Some(1),
    /// });
    /// let _connection = limiter.try_acquire(Uuid::now_v7())?;
    /// assert!(limiter.try_acquire(Uuid::now_v7()).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn try_acquire(&self, principal: Uuid) -> Result<SseConnection, ApiError> {
        let mut counts = self.lock();

        if self.limits.global.is_some_and(|max| counts.total >= max) {
            return Err(ApiError::SseConnectionLimit);
        }
        let held = counts.per_principal.get(&principal).copied().unwrap_or(0);
        if self.limits.per_principal.is_some_and(|max| held >= max) {
            return Err(ApiError::SseConnectionLimit);
        }

        counts.total += 1;
        counts.per_principal.insert(principal, held + 1);

        Ok(SseConnection {
            limiter: self.clone(),
            principal,
        })
    }

    /// Streams currently open on the whole server.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::sse_limiter::SseLimiter;
    ///
    /// assert_eq!(SseLimiter::default().open_connections(), 0);
    /// ```
    pub fn open_connections(&self) -> usize {
        self.lock().total
    }

    // A panic while holding the lock leaves the counters consistent (every
    // update is a single statement), so a poisoned lock is still usable.
    fn lock(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn release(&self, principal: Uuid) {
        let mut counts = self.lock();
        counts.total = counts.total.saturating_sub(1);
        if let Some(held) = counts.per_principal.get_mut(&principal) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                counts.per_principal.remove(&principal);
            }
        }
    }
}

/// A reserved SSE stream slot, released when dropped.
///
/// Keep it alive for as long as the stream it guards: move it into the
/// stream so the slot is freed when the client disconnects or the stream
/// ends.
///
/// # Examples
///
/// ```
/// use ironflow_api::sse_limiter::SseLimiter;
/// use uuid::Uuid;
///
/// # fn example() -> Result<(), ironflow_api::error::ApiError> {
/// let limiter = SseLimiter::default();
/// let connection = limiter.try_acquire(Uuid::now_v7())?;
/// assert_eq!(limiter.open_connections(), 1);
/// drop(connection);
/// assert_eq!(limiter.open_connections(), 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct SseConnection {
    limiter: SseLimiter,
    principal: Uuid,
}

impl Drop for SseConnection {
    fn drop(&mut self) {
        self.limiter.release(self.principal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(per_principal: Option<usize>, global: Option<usize>) -> SseLimiter {
        SseLimiter::new(SseLimits {
            per_principal,
            global,
        })
    }

    #[test]
    fn per_principal_cap_refuses_the_next_stream() {
        let limiter = SseLimiter::default();
        let user = Uuid::now_v7();

        let held: Vec<_> = (0..DEFAULT_SSE_MAX_PER_PRINCIPAL)
            .map(|_| limiter.try_acquire(user).expect("under the cap"))
            .collect();

        let refused = limiter.try_acquire(user);
        assert!(matches!(refused, Err(ApiError::SseConnectionLimit)));
        let other = limiter.try_acquire(Uuid::now_v7());
        assert!(other.is_ok(), "another principal is unaffected");
        assert_eq!(held.len(), DEFAULT_SSE_MAX_PER_PRINCIPAL);
    }

    #[test]
    fn global_cap_refuses_every_principal() {
        let limiter = limiter(None, Some(2));
        let _a = limiter.try_acquire(Uuid::now_v7()).expect("first");
        let _b = limiter.try_acquire(Uuid::now_v7()).expect("second");

        let refused = limiter.try_acquire(Uuid::now_v7());
        assert!(matches!(refused, Err(ApiError::SseConnectionLimit)));
    }

    #[test]
    fn dropping_a_connection_frees_its_slot() {
        let limiter = limiter(Some(1), Some(1));
        let user = Uuid::now_v7();

        let connection = limiter.try_acquire(user).expect("first");
        assert!(limiter.try_acquire(user).is_err());
        drop(connection);
        assert!(limiter.try_acquire(user).is_ok());
    }

    #[test]
    fn unlimited_when_limits_are_none() {
        let limiter = limiter(None, None);
        let user = Uuid::now_v7();

        let held: Vec<_> = (0..50)
            .map(|_| limiter.try_acquire(user).expect("unlimited"))
            .collect();
        assert_eq!(limiter.open_connections(), held.len());
    }

    #[test]
    fn zero_limit_refuses_everything() {
        let limiter = limiter(Some(0), None);
        assert!(limiter.try_acquire(Uuid::now_v7()).is_err());
    }

    #[test]
    fn counters_return_to_zero() {
        let limiter = SseLimiter::default();
        let user = Uuid::now_v7();

        let a = limiter.try_acquire(user).expect("a");
        let b = limiter.try_acquire(user).expect("b");
        let c = limiter.try_acquire(Uuid::now_v7()).expect("c");
        assert_eq!(limiter.open_connections(), 3);
        drop((a, b, c));

        assert_eq!(limiter.open_connections(), 0);
        assert!(limiter.lock().per_principal.is_empty());
    }

    #[test]
    fn clones_share_counters() {
        let limiter = limiter(Some(1), None);
        let clone = limiter.clone();
        let user = Uuid::now_v7();

        let _connection = limiter.try_acquire(user).expect("first");
        assert!(clone.try_acquire(user).is_err());
    }

    #[test]
    fn connection_is_send_and_static() {
        fn assert_send_static<T: Send + 'static>() {}
        assert_send_static::<SseConnection>();
    }
}
