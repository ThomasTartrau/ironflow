//! Worker lease request/response types for the internal routes.

use std::time::Duration;

use chrono::{DateTime, Utc};
use ironflow_store::entities::LeaseRequest;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

/// Default lease duration when the worker does not specify one: three missed
/// refreshes (30 s each) before a run becomes recoverable.
pub const DEFAULT_LEASE_TTL_SECS: u64 = 90;

/// Longest lease a worker may request, in seconds.
///
/// A lease longer than an hour would keep a dead worker's run stuck for that
/// long, which defeats the purpose of the lease.
pub const MAX_LEASE_TTL_SECS: u64 = 3600;

/// Body of `POST /api/v1/internal/runs/{id}/lease`.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::RenewLeaseRequest;
///
/// let body: RenewLeaseRequest = serde_json::from_str(
///     r#"{"worker_id": "worker-1", "lease_ttl_secs": 90}"#,
/// )?;
/// assert_eq!(body.worker_id, "worker-1");
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct RenewLeaseRequest {
    /// Identifier of the worker that currently owns the run.
    pub worker_id: String,
    /// New lease duration in seconds. Defaults to [`DEFAULT_LEASE_TTL_SECS`].
    #[serde(default)]
    pub lease_ttl_secs: Option<u64>,
}

/// Response of a successful lease renewal.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_api::entities::RenewLeaseResponse;
///
/// let resp = RenewLeaseResponse { lease_expires_at: Utc::now() };
/// assert!(serde_json::to_string(&resp)?.contains("lease_expires_at"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct RenewLeaseResponse {
    /// New expiry of the lease, as computed by the store.
    pub lease_expires_at: DateTime<Utc>,
}

/// Build a [`LeaseRequest`] from raw worker input.
///
/// Returns `Ok(None)` when `worker_id` is absent — the caller takes the run
/// without a lease, which is how workers from an older release keep working.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if `worker_id` is blank or if
/// `lease_ttl_secs` is zero or above [`MAX_LEASE_TTL_SECS`].
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::validate_lease_ttl;
///
/// let lease = validate_lease_ttl(Some("worker-1".to_string()), Some(90))?;
/// assert_eq!(lease.unwrap().ttl.as_secs(), 90);
/// assert!(validate_lease_ttl(None, None)?.is_none());
/// assert!(validate_lease_ttl(Some("worker-1".to_string()), Some(0)).is_err());
/// # Ok::<(), ironflow_api::error::ApiError>(())
/// ```
pub fn validate_lease_ttl(
    worker_id: Option<String>,
    lease_ttl_secs: Option<u64>,
) -> Result<Option<LeaseRequest>, ApiError> {
    let Some(worker_id) = worker_id else {
        return Ok(None);
    };

    let worker_id = worker_id.trim().to_string();
    if worker_id.is_empty() {
        return Err(ApiError::BadRequest("worker_id must not be blank".into()));
    }

    let ttl_secs = lease_ttl_secs.unwrap_or(DEFAULT_LEASE_TTL_SECS);
    if ttl_secs == 0 || ttl_secs > MAX_LEASE_TTL_SECS {
        return Err(ApiError::BadRequest(format!(
            "lease_ttl_secs must be between 1 and {MAX_LEASE_TTL_SECS}"
        )));
    }

    Ok(Some(LeaseRequest {
        worker_id,
        ttl: Duration::from_secs(ttl_secs),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_worker_id_means_no_lease() {
        assert!(validate_lease_ttl(None, Some(90)).unwrap().is_none());
    }

    #[test]
    fn default_ttl_is_applied() {
        let lease = validate_lease_ttl(Some("worker-1".to_string()), None)
            .unwrap()
            .unwrap();
        assert_eq!(lease.ttl.as_secs(), DEFAULT_LEASE_TTL_SECS);
        assert_eq!(lease.worker_id, "worker-1");
    }

    #[test]
    fn worker_id_is_trimmed() {
        let lease = validate_lease_ttl(Some("  worker-1  ".to_string()), None)
            .unwrap()
            .unwrap();
        assert_eq!(lease.worker_id, "worker-1");
    }

    #[test]
    fn blank_worker_id_is_rejected() {
        let err = validate_lease_ttl(Some("   ".to_string()), None).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn zero_ttl_is_rejected() {
        let err = validate_lease_ttl(Some("worker-1".to_string()), Some(0)).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn ttl_above_max_is_rejected() {
        let err = validate_lease_ttl(Some("worker-1".to_string()), Some(MAX_LEASE_TTL_SECS + 1))
            .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn max_ttl_is_accepted() {
        let lease = validate_lease_ttl(Some("worker-1".to_string()), Some(MAX_LEASE_TTL_SECS))
            .unwrap()
            .unwrap();
        assert_eq!(lease.ttl.as_secs(), MAX_LEASE_TTL_SECS);
    }
}
