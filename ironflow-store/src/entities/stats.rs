//! [`RunStats`] — aggregated statistics across all runs.

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
