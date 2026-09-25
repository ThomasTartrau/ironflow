//! Statistics DTOs.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ironflow_store::entities::{HistoryGranularity, HistoryPeriod, StatsHistoryBucket};
use ironflow_store::models::RunStatus;

use super::run::parse_label_param;

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
    /// Number of active runs: pending, running, retrying, awaiting approval
    /// or sleeping.
    pub active_runs: u64,
    /// Number of runs awaiting approval. A subset of `active_runs`.
    pub awaiting_approval_runs: u64,
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
    /// Filter by workflow name (case-insensitive substring match, same as
    /// `GET /api/v1/runs`). Omit to aggregate all workflows.
    pub workflow: Option<String>,
    /// Time period to query. Defaults to `7d`.
    pub period: Option<HistoryPeriod>,
    /// Bucket granularity. Auto-derived from period when omitted.
    pub granularity: Option<HistoryGranularity>,
    /// Filter by run status.
    pub status: Option<RunStatus>,
    /// Filter by step presence (only applies to completed/cancelled runs).
    /// Non-terminal runs (pending, running, etc.) are always included.
    /// When `true`, only count completed/cancelled runs that have steps.
    /// When `false`, only count completed/cancelled runs without steps.
    pub has_steps: Option<bool>,
    /// Filter by labels. Comma-separated `key:value` pairs.
    pub label: Option<String>,
    /// Filter by author: the user ID that triggered the run.
    ///
    /// Also matches runs triggered by one of that user's API keys.
    pub created_by: Option<Uuid>,
}

impl StatsHistoryQuery {
    /// Parse the comma-separated `label` param into a `HashMap`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::entities::StatsHistoryQuery;
    ///
    /// let query = StatsHistoryQuery {
    ///     workflow: None,
    ///     period: None,
    ///     granularity: None,
    ///     status: None,
    ///     has_steps: None,
    ///     label: Some("env:prod,team:core".to_string()),
    ///     created_by: None,
    /// };
    /// let labels = query.parse_labels().unwrap_or_default();
    /// assert_eq!(labels.get("env").map(String::as_str), Some("prod"));
    /// assert_eq!(labels.len(), 2);
    /// ```
    pub fn parse_labels(&self) -> Option<HashMap<String, String>> {
        parse_label_param(&self.label)
    }
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
    /// Every bucket of the period, zero-filled, sorted by time ascending.
    pub buckets: Vec<StatsHistoryBucketResponse>,
}

/// One time bucket in the history response.
///
/// Runs are assigned to the bucket of their creation time and counted under
/// their current status. Each status has its own counter, so the counters add
/// up to the number of runs created in the bucket.
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
    /// Number of runs created in this bucket and currently completed.
    /// Strict: runs in the `warning` state are counted in `warning`.
    pub completed: u64,
    /// Number of runs created in this bucket and currently in `warning`.
    pub warning: u64,
    /// Number of runs created in this bucket and currently failed.
    pub failed: u64,
    /// Number of runs created in this bucket and currently cancelled.
    pub cancelled: u64,
    /// Number of runs created in this bucket and currently pending.
    pub pending: u64,
    /// Number of runs created in this bucket and currently running.
    pub running: u64,
    /// Number of runs created in this bucket and currently retrying.
    pub retrying: u64,
    /// Number of runs created in this bucket and currently awaiting approval.
    pub awaiting_approval: u64,
    /// Number of runs created in this bucket and currently sleeping.
    pub sleeping: u64,
    /// Success rate: (completed + warning) / (completed + warning + failed),
    /// as a percentage. `null` when the bucket has no completed, warning or
    /// failed run.
    pub success_rate_percent: Option<f64>,
    /// Average duration in milliseconds.
    pub avg_duration_ms: u64,
    /// 95th percentile duration in milliseconds.
    pub p95_duration_ms: u64,
    /// Total cost in USD.
    pub total_cost_usd: Decimal,
}

impl From<StatsHistoryBucket> for StatsHistoryBucketResponse {
    fn from(b: StatsHistoryBucket) -> Self {
        let success_rate_percent = b.success_rate_percent();
        StatsHistoryBucketResponse {
            time: b.time,
            completed: b.completed,
            warning: b.warning,
            failed: b.failed,
            cancelled: b.cancelled,
            pending: b.pending,
            running: b.running,
            retrying: b.retrying,
            awaiting_approval: b.awaiting_approval,
            sleeping: b.sleeping,
            success_rate_percent,
            avg_duration_ms: b.avg_duration_ms,
            p95_duration_ms: b.p95_duration_ms,
            total_cost_usd: b.total_cost_usd,
        }
    }
}
