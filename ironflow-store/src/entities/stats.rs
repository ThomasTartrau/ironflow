//! [`RunStats`] — aggregated statistics across all runs.
//! [`StatsHistoryBucket`] — time-bucketed statistics for trend charts.

use std::fmt;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

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
///     active_runs: 0,
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
    /// Runs in an active state: `Pending`, `Running`, or `Retrying`.
    pub active_runs: u64,
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
}

/// Filter for historical statistics queries.
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
/// };
/// assert_eq!(filter.period.to_string(), "7d");
/// ```
#[derive(Debug, Clone)]
pub struct StatsHistoryFilter {
    /// Filter by workflow name (exact match). `None` means all workflows.
    pub workflow_name: Option<String>,
    /// How far back to query.
    pub period: HistoryPeriod,
    /// Size of each time bucket.
    pub granularity: HistoryGranularity,
}

/// One time bucket of aggregated run statistics.
///
/// Each bucket covers a time range determined by the granularity
/// (1 hour, 1 day, or 1 week).
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
///     failed: 3,
///     cancelled: 1,
///     avg_duration_ms: 45000,
///     p95_duration_ms: 120000,
///     total_cost_usd: Decimal::new(123, 2),
/// };
/// assert_eq!(bucket.completed, 42);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsHistoryBucket {
    /// Start of the time bucket.
    pub time: DateTime<Utc>,
    /// Number of runs that completed in this bucket.
    pub completed: u64,
    /// Number of runs that failed in this bucket.
    pub failed: u64,
    /// Number of runs that were cancelled in this bucket.
    pub cancelled: u64,
    /// Average duration in milliseconds of terminal runs in this bucket.
    pub avg_duration_ms: u64,
    /// 95th percentile duration in milliseconds of terminal runs in this bucket.
    pub p95_duration_ms: u64,
    /// Total cost in USD of all runs in this bucket.
    pub total_cost_usd: Decimal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zeros() {
        let stats = RunStats::default();
        assert_eq!(stats.total_runs, 0);
        assert_eq!(stats.completed_runs, 0);
        assert_eq!(stats.failed_runs, 0);
        assert_eq!(stats.cancelled_runs, 0);
        assert_eq!(stats.active_runs, 0);
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
            active_runs: 0,
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
        assert_eq!(stats.total_cost_usd, back.total_cost_usd);
        assert_eq!(stats.total_duration_ms, back.total_duration_ms);
    }
}
