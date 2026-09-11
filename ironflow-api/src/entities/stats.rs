//! Statistics DTOs.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use ironflow_store::entities::{HistoryGranularity, HistoryPeriod};

/// Aggregate statistics response.
///
/// Computed from all runs in the store.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::StatsResponse;
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    /// Total number of runs.
    pub total_runs: u64,
    /// Number of completed runs.
    pub completed_runs: u64,
    /// Number of failed runs.
    pub failed_runs: u64,
    /// Number of cancelled runs.
    pub cancelled_runs: u64,
    /// Number of pending or running runs.
    pub active_runs: u64,
    /// Success rate: completed / (completed + failed), as a percentage.
    pub success_rate_percent: f64,
    /// Aggregated cost across all runs in USD.
    pub total_cost_usd: Decimal,
    /// Aggregated duration across all runs in milliseconds.
    pub total_duration_ms: u64,
}

/// Query parameters for `GET /api/v1/stats/history`.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::StatsHistoryQuery;
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[derive(Debug, Deserialize)]
pub struct StatsHistoryQuery {
    /// Filter by workflow name. Omit to aggregate all workflows.
    pub workflow: Option<String>,
    /// Time period to query. Defaults to `7d`.
    pub period: Option<HistoryPeriod>,
    /// Bucket granularity. Auto-derived from period when omitted.
    pub granularity: Option<HistoryGranularity>,
}

/// Time-bucketed historical statistics response.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::StatsHistoryResponse;
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct StatsHistoryResponse {
    /// The period that was queried.
    pub period: HistoryPeriod,
    /// The granularity of each bucket.
    pub granularity: HistoryGranularity,
    /// Workflow name filter, if applied.
    pub workflow: Option<String>,
    /// Aggregated buckets, sorted by time ascending.
    pub buckets: Vec<StatsHistoryBucketResponse>,
}

/// One time bucket in the history response.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::StatsHistoryBucketResponse;
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct StatsHistoryBucketResponse {
    /// Start of the time bucket.
    pub time: DateTime<Utc>,
    /// Number of completed runs in this bucket.
    pub completed: u64,
    /// Number of failed runs in this bucket.
    pub failed: u64,
    /// Number of cancelled runs in this bucket.
    pub cancelled: u64,
    /// Average duration in milliseconds.
    pub avg_duration_ms: u64,
    /// 95th percentile duration in milliseconds.
    pub p95_duration_ms: u64,
    /// Total cost in USD.
    pub total_cost_usd: Decimal,
}
