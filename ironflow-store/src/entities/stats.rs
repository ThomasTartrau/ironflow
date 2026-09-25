//! [`RunStats`] — aggregated statistics across all runs.
//! [`StatsHistoryBucket`] — time-bucketed statistics for trend charts.

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{RunFilter, RunStatus};

/// Aggregated statistics for all runs in the store.
///
/// Computed efficiently by the store implementation (single SQL query in PostgreSQL,
/// in-memory aggregation in InMemoryStore).
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::RunStats;
/// use rust_decimal::Decimal;
///
/// let stats = RunStats {
///     total_runs: 100,
///     completed_runs: 80,
///     failed_runs: 15,
///     cancelled_runs: 5,
///     active_runs: 3,
///     awaiting_approval_runs: 1,
///     total_cost_usd: Decimal::new(4250, 2),
///     total_duration_ms: 3600000,
/// };
/// assert_eq!(stats.total_runs, 100);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunStats {
    /// Total number of runs ever created.
    pub total_runs: u64,
    /// Runs that reached the `Completed` state.
    pub completed_runs: u64,
    /// Runs that reached the `Failed` state.
    pub failed_runs: u64,
    /// Runs that reached the `Cancelled` state.
    pub cancelled_runs: u64,
    /// Runs in an active state: `Pending`, `Running`, `Retrying`,
    /// `AwaitingApproval` or `Sleeping`.
    pub active_runs: u64,
    /// Runs in the `AwaitingApproval` state. A subset of
    /// [`active_runs`](Self::active_runs).
    #[serde(default)]
    pub awaiting_approval_runs: u64,
    /// Total cost in USD across all runs.
    pub total_cost_usd: Decimal,
    /// Total execution time in milliseconds across all runs.
    pub total_duration_ms: u64,
}

/// Time period for historical statistics queries.
///
/// Controls how far back the query reaches from the current time.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::HistoryPeriod;
///
/// let period = HistoryPeriod::SevenDays;
/// assert_eq!(period.to_string(), "7d");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryPeriod {
    /// Last 24 hours.
    #[serde(rename = "24h")]
    TwentyFourHours,
    /// Last 7 days.
    #[default]
    #[serde(rename = "7d")]
    SevenDays,
    /// Last 30 days.
    #[serde(rename = "30d")]
    ThirtyDays,
    /// Last 90 days.
    #[serde(rename = "90d")]
    NinetyDays,
}

impl fmt::Display for HistoryPeriod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TwentyFourHours => write!(f, "24h"),
            Self::SevenDays => write!(f, "7d"),
            Self::ThirtyDays => write!(f, "30d"),
            Self::NinetyDays => write!(f, "90d"),
        }
    }
}

impl HistoryPeriod {
    /// Returns the default granularity for this period.
    ///
    /// - `24h` -> `1h`
    /// - `7d` -> `1d`
    /// - `30d` -> `1d`
    /// - `90d` -> `1w`
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::{HistoryPeriod, HistoryGranularity};
    ///
    /// assert_eq!(HistoryPeriod::TwentyFourHours.default_granularity(), HistoryGranularity::OneHour);
    /// assert_eq!(HistoryPeriod::NinetyDays.default_granularity(), HistoryGranularity::OneWeek);
    /// ```
    pub fn default_granularity(&self) -> HistoryGranularity {
        match self {
            Self::TwentyFourHours => HistoryGranularity::OneHour,
            Self::SevenDays | Self::ThirtyDays => HistoryGranularity::OneDay,
            Self::NinetyDays => HistoryGranularity::OneWeek,
        }
    }

    /// Returns the number of hours this period spans.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::HistoryPeriod;
    ///
    /// assert_eq!(HistoryPeriod::TwentyFourHours.hours(), 24);
    /// assert_eq!(HistoryPeriod::SevenDays.hours(), 168);
    /// ```
    pub fn hours(&self) -> i64 {
        match self {
            Self::TwentyFourHours => 24,
            Self::SevenDays => 7 * 24,
            Self::ThirtyDays => 30 * 24,
            Self::NinetyDays => 90 * 24,
        }
    }
}

/// Time bucket granularity for historical statistics.
///
/// Controls the size of each time bucket in the response.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::HistoryGranularity;
///
/// let gran = HistoryGranularity::OneDay;
/// assert_eq!(gran.to_string(), "1d");
/// assert_eq!(gran.pg_interval(), "day");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryGranularity {
    /// One-hour buckets.
    #[serde(rename = "1h")]
    OneHour,
    /// One-day buckets.
    #[serde(rename = "1d")]
    OneDay,
    /// One-week buckets.
    #[serde(rename = "1w")]
    OneWeek,
}

impl fmt::Display for HistoryGranularity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OneHour => write!(f, "1h"),
            Self::OneDay => write!(f, "1d"),
            Self::OneWeek => write!(f, "1w"),
        }
    }
}

impl HistoryGranularity {
    /// Returns the PostgreSQL `date_trunc` interval name.
    ///
    /// The store applies it to timestamps converted to UTC, so buckets match
    /// [`bucket_start`](Self::bucket_start) regardless of the session time zone.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::HistoryGranularity;
    ///
    /// assert_eq!(HistoryGranularity::OneHour.pg_interval(), "hour");
    /// ```
    pub fn pg_interval(&self) -> &'static str {
        match self {
            Self::OneHour => "hour",
            Self::OneDay => "day",
            Self::OneWeek => "week",
        }
    }

    /// Returns the number of seconds in one bucket of this granularity.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::HistoryGranularity;
    ///
    /// assert_eq!(HistoryGranularity::OneHour.seconds(), 3600);
    /// ```
    pub fn seconds(&self) -> i64 {
        match self {
            Self::OneHour => 3600,
            Self::OneDay => 86400,
            Self::OneWeek => 604800,
        }
    }

    /// Returns the start of the bucket containing `time`.
    ///
    /// All boundaries are UTC: hours start at `:00`, days at `00:00`, and
    /// weeks on Monday `00:00`. Timestamps before the Unix epoch are handled
    /// with Euclidean division, so they still map to the bucket start at or
    /// before `time`.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{DateTime, ParseError, Utc};
    /// use ironflow_store::entities::HistoryGranularity;
    ///
    /// // 2026-09-24 is a Thursday.
    /// let time: DateTime<Utc> = "2026-09-24T13:45:00Z".parse()?;
    /// let hour: DateTime<Utc> = "2026-09-24T13:00:00Z".parse()?;
    /// let day: DateTime<Utc> = "2026-09-24T00:00:00Z".parse()?;
    /// let week: DateTime<Utc> = "2026-09-21T00:00:00Z".parse()?;
    /// assert_eq!(HistoryGranularity::OneHour.bucket_start(time), hour);
    /// assert_eq!(HistoryGranularity::OneDay.bucket_start(time), day);
    /// assert_eq!(HistoryGranularity::OneWeek.bucket_start(time), week);
    /// # Ok::<(), ParseError>(())
    /// ```
    pub fn bucket_start(&self, time: DateTime<Utc>) -> DateTime<Utc> {
        let secs = time.timestamp();
        let step = self.seconds();
        let start = match self {
            Self::OneHour | Self::OneDay => secs.div_euclid(step) * step,
            Self::OneWeek => {
                // The Unix epoch is a Thursday; 1970-01-05 is the first Monday.
                let offset = 4 * 86_400;
                (secs - offset).div_euclid(step) * step + offset
            }
        };
        DateTime::from_timestamp(start, 0).unwrap_or(time)
    }
}

/// Filter for historical statistics queries.
///
/// The run filters (`workflow_name`, `status`, `has_steps`, `labels`,
/// `created_by_user_id`) have the exact semantics of [`RunFilter`], so the
/// history matches what `get_stats` and `list_runs` return for the same filter.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{StatsHistoryFilter, HistoryPeriod, HistoryGranularity};
///
/// let filter = StatsHistoryFilter {
///     workflow_name: Some("deploy".to_string()),
///     period: HistoryPeriod::SevenDays,
///     granularity: HistoryGranularity::OneDay,
///     ..StatsHistoryFilter::default()
/// };
/// assert_eq!(filter.period.to_string(), "7d");
/// ```
#[derive(Debug, Clone)]
pub struct StatsHistoryFilter {
    /// Filter by workflow name (case-insensitive substring match).
    /// `None` means all workflows.
    pub workflow_name: Option<String>,
    /// Filter by run status.
    pub status: Option<RunStatus>,
    /// When `Some(true)`, only include runs that have at least one step.
    /// When `Some(false)`, only include runs with no steps.
    /// When `None`, no filtering on steps.
    pub has_steps: Option<bool>,
    /// Filter by label key-value pair. Only include runs that have ALL specified labels.
    pub labels: Option<HashMap<String, String>>,
    /// Filter by author. Matches runs created by this user directly, and runs
    /// created by one of this user's API keys.
    pub created_by_user_id: Option<Uuid>,
    /// How far back to query.
    pub period: HistoryPeriod,
    /// Size of each time bucket.
    pub granularity: HistoryGranularity,
}

impl Default for StatsHistoryFilter {
    fn default() -> Self {
        let period = HistoryPeriod::default();
        Self {
            workflow_name: None,
            status: None,
            has_steps: None,
            labels: None,
            created_by_user_id: None,
            period,
            granularity: period.default_granularity(),
        }
    }
}

impl StatsHistoryFilter {
    /// Returns the [`RunFilter`] equivalent of this filter, without any time range.
    ///
    /// The time range is derived from [`period`](Self::period) by the store.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::{RunStatus, StatsHistoryFilter};
    ///
    /// let filter = StatsHistoryFilter {
    ///     status: Some(RunStatus::Failed),
    ///     ..StatsHistoryFilter::default()
    /// };
    /// let run_filter = filter.to_run_filter();
    /// assert_eq!(run_filter.status, Some(RunStatus::Failed));
    /// assert!(run_filter.created_after.is_none());
    /// ```
    pub fn to_run_filter(&self) -> RunFilter {
        RunFilter {
            workflow_name: self.workflow_name.clone(),
            status: self.status,
            created_after: None,
            created_before: None,
            has_steps: self.has_steps,
            labels: self.labels.clone(),
            created_by_user_id: self.created_by_user_id,
        }
    }
}

/// One time bucket of aggregated run statistics.
///
/// Each bucket covers a time range determined by the granularity
/// (1 hour, 1 day, or 1 week, UTC boundaries, weeks starting on Monday).
///
/// Runs are assigned to the bucket of their creation time and counted under
/// their current status. Every status has its own counter, so the counters
/// add up to the number of runs created in the bucket and no run is counted
/// twice.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use rust_decimal::Decimal;
/// use ironflow_store::entities::StatsHistoryBucket;
///
/// let bucket = StatsHistoryBucket {
///     time: Utc::now(),
///     completed: 42,
///     warning: 2,
///     failed: 3,
///     cancelled: 1,
///     running: 1,
///     avg_duration_ms: 45000,
///     p95_duration_ms: 120000,
///     total_cost_usd: Decimal::new(123, 2),
///     ..StatsHistoryBucket::default()
/// };
/// assert_eq!(bucket.completed, 42);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatsHistoryBucket {
    /// Start of the time bucket.
    pub time: DateTime<Utc>,
    /// Runs created in this bucket and currently `Completed`.
    ///
    /// Strict: runs in the `Warning` state are counted in
    /// [`warning`](Self::warning), not here.
    pub completed: u64,
    /// Runs created in this bucket and currently `Warning`.
    pub warning: u64,
    /// Runs created in this bucket and currently `Failed`.
    pub failed: u64,
    /// Runs created in this bucket and currently `Cancelled`.
    pub cancelled: u64,
    /// Runs created in this bucket and currently `Pending`.
    pub pending: u64,
    /// Runs created in this bucket and currently `Running`.
    pub running: u64,
    /// Runs created in this bucket and currently `Retrying`.
    pub retrying: u64,
    /// Runs created in this bucket and currently `AwaitingApproval`.
    pub awaiting_approval: u64,
    /// Runs created in this bucket and currently `Sleeping`.
    pub sleeping: u64,
    /// Average duration in milliseconds of terminal runs in this bucket.
    pub avg_duration_ms: u64,
    /// 95th percentile duration in milliseconds of terminal runs in this bucket.
    pub p95_duration_ms: u64,
    /// Total cost in USD of all runs in this bucket.
    pub total_cost_usd: Decimal,
}

impl StatsHistoryBucket {
    /// Returns the success rate of the bucket, in percent.
    ///
    /// Computed as `(completed + warning) / (completed + warning + failed) * 100`,
    /// consistent with `get_stats` which counts `Warning` as a success.
    /// Returns `None` when the bucket has no completed, warning or failed run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::StatsHistoryBucket;
    ///
    /// let bucket = StatsHistoryBucket {
    ///     completed: 1,
    ///     failed: 1,
    ///     ..StatsHistoryBucket::default()
    /// };
    /// assert_eq!(bucket.success_rate_percent(), Some(50.0));
    /// assert_eq!(StatsHistoryBucket::default().success_rate_percent(), None);
    /// ```
    pub fn success_rate_percent(&self) -> Option<f64> {
        let successes = self.completed + self.warning;
        let denominator = successes + self.failed;
        if denominator == 0 {
            return None;
        }
        Some(successes as f64 / denominator as f64 * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Duration, Timelike, Weekday};

    use super::*;

    #[test]
    fn default_is_zeros() {
        let stats = RunStats::default();
        assert_eq!(stats.total_runs, 0);
        assert_eq!(stats.completed_runs, 0);
        assert_eq!(stats.failed_runs, 0);
        assert_eq!(stats.cancelled_runs, 0);
        assert_eq!(stats.active_runs, 0);
        assert_eq!(stats.awaiting_approval_runs, 0);
        assert_eq!(stats.total_cost_usd, Decimal::ZERO);
        assert_eq!(stats.total_duration_ms, 0);
    }

    #[test]
    fn serde_roundtrip() {
        let stats = RunStats {
            total_runs: 100,
            completed_runs: 80,
            failed_runs: 15,
            cancelled_runs: 5,
            active_runs: 4,
            awaiting_approval_runs: 2,
            total_cost_usd: Decimal::new(4250, 2),
            total_duration_ms: 3600000,
        };
        let json = serde_json::to_string(&stats).expect("serialize");
        let back: RunStats = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(stats.total_runs, back.total_runs);
        assert_eq!(stats.completed_runs, back.completed_runs);
        assert_eq!(stats.failed_runs, back.failed_runs);
        assert_eq!(stats.cancelled_runs, back.cancelled_runs);
        assert_eq!(stats.active_runs, back.active_runs);
        assert_eq!(stats.awaiting_approval_runs, back.awaiting_approval_runs);
        assert_eq!(stats.total_cost_usd, back.total_cost_usd);
        assert_eq!(stats.total_duration_ms, back.total_duration_ms);
    }

    #[test]
    fn run_stats_awaiting_approval_defaults_when_missing() {
        let json = r#"{"total_runs":1,"completed_runs":0,"failed_runs":0,"cancelled_runs":0,"active_runs":1,"total_cost_usd":"0","total_duration_ms":0}"#;
        let stats: RunStats = serde_json::from_str(json).expect("deserialize");
        assert_eq!(stats.awaiting_approval_runs, 0);
    }

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().expect("valid timestamp")
    }

    #[test]
    fn bucket_start_hour() {
        assert_eq!(
            HistoryGranularity::OneHour.bucket_start(utc("2026-09-24T13:45:12Z")),
            utc("2026-09-24T13:00:00Z")
        );
        assert_eq!(
            HistoryGranularity::OneHour.bucket_start(utc("2026-09-24T13:00:00Z")),
            utc("2026-09-24T13:00:00Z")
        );
    }

    #[test]
    fn bucket_start_day() {
        assert_eq!(
            HistoryGranularity::OneDay.bucket_start(utc("2026-09-24T23:59:59Z")),
            utc("2026-09-24T00:00:00Z")
        );
    }

    #[test]
    fn bucket_start_week_thursday_maps_to_previous_monday() {
        assert_eq!(
            HistoryGranularity::OneWeek.bucket_start(utc("2026-09-24T13:45:00Z")),
            utc("2026-09-21T00:00:00Z")
        );
    }

    #[test]
    fn bucket_start_week_sunday_maps_to_previous_monday() {
        assert_eq!(
            HistoryGranularity::OneWeek.bucket_start(utc("2026-09-27T23:59:59Z")),
            utc("2026-09-21T00:00:00Z")
        );
    }

    #[test]
    fn bucket_start_week_monday_midnight_maps_to_itself() {
        assert_eq!(
            HistoryGranularity::OneWeek.bucket_start(utc("2026-09-21T00:00:00Z")),
            utc("2026-09-21T00:00:00Z")
        );
    }

    #[test]
    fn bucket_start_week_is_always_monday_midnight() {
        let mut time = utc("2026-01-01T07:30:00Z");
        for _ in 0..30 {
            let start = HistoryGranularity::OneWeek.bucket_start(time);
            assert_eq!(start.weekday(), Weekday::Mon);
            assert_eq!(start.hour(), 0);
            assert_eq!(start.minute(), 0);
            assert!(start <= time);
            time += Duration::hours(29);
        }
    }

    #[test]
    fn bucket_start_pre_epoch() {
        assert_eq!(
            HistoryGranularity::OneHour.bucket_start(utc("1969-12-31T23:30:00Z")),
            utc("1969-12-31T23:00:00Z")
        );
        assert_eq!(
            HistoryGranularity::OneDay.bucket_start(utc("1969-12-31T12:00:00Z")),
            utc("1969-12-31T00:00:00Z")
        );
        // 1969-12-31 is a Wednesday; its week starts on Monday 1969-12-29.
        assert_eq!(
            HistoryGranularity::OneWeek.bucket_start(utc("1969-12-31T12:00:00Z")),
            utc("1969-12-29T00:00:00Z")
        );
    }

    #[test]
    fn success_rate_none_on_empty_bucket() {
        assert_eq!(StatsHistoryBucket::default().success_rate_percent(), None);
    }

    #[test]
    fn success_rate_none_with_only_running_and_cancelled() {
        let bucket = StatsHistoryBucket {
            running: 3,
            cancelled: 2,
            pending: 1,
            ..StatsHistoryBucket::default()
        };
        assert_eq!(bucket.success_rate_percent(), None);
    }

    #[test]
    fn success_rate_counts_warning_as_success() {
        let bucket = StatsHistoryBucket {
            warning: 2,
            ..StatsHistoryBucket::default()
        };
        assert_eq!(bucket.success_rate_percent(), Some(100.0));
    }

    #[test]
    fn success_rate_half() {
        let bucket = StatsHistoryBucket {
            completed: 1,
            failed: 1,
            ..StatsHistoryBucket::default()
        };
        assert_eq!(bucket.success_rate_percent(), Some(50.0));
    }

    #[test]
    fn stats_history_filter_default() {
        let filter = StatsHistoryFilter::default();
        assert_eq!(filter.period, HistoryPeriod::SevenDays);
        assert_eq!(filter.granularity, HistoryGranularity::OneDay);
        assert!(filter.workflow_name.is_none());
        assert!(filter.status.is_none());
        assert!(filter.has_steps.is_none());
        assert!(filter.labels.is_none());
        assert!(filter.created_by_user_id.is_none());
    }

    #[test]
    fn stats_history_filter_to_run_filter_copies_filters() {
        let user_id = Uuid::now_v7();
        let labels = HashMap::from([("env".to_string(), "prod".to_string())]);
        let filter = StatsHistoryFilter {
            workflow_name: Some("deploy".to_string()),
            status: Some(RunStatus::Running),
            has_steps: Some(true),
            labels: Some(labels.clone()),
            created_by_user_id: Some(user_id),
            period: HistoryPeriod::NinetyDays,
            granularity: HistoryGranularity::OneWeek,
        };
        let run_filter = filter.to_run_filter();
        assert_eq!(run_filter.workflow_name.as_deref(), Some("deploy"));
        assert_eq!(run_filter.status, Some(RunStatus::Running));
        assert_eq!(run_filter.has_steps, Some(true));
        assert_eq!(run_filter.labels, Some(labels));
        assert_eq!(run_filter.created_by_user_id, Some(user_id));
        assert!(run_filter.created_after.is_none());
        assert!(run_filter.created_before.is_none());
    }
}
