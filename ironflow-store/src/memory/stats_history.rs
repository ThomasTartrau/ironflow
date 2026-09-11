//! In-memory aggregation for [`RunStore::get_stats_history`].

use std::collections::HashMap;

use chrono::DateTime;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::entities::{Run, RunStatus, StatsHistoryBucket};

/// Aggregate runs into time-bucketed statistics.
///
/// Filters runs by time range and optional workflow name, groups them into
/// buckets of `granularity_secs` width, and computes per-bucket aggregates.
/// Returns buckets sorted by time ascending; empty periods are omitted.
pub(crate) fn aggregate_history_buckets<'a>(
    runs: impl Iterator<Item = &'a Run>,
    workflow_name: &Option<String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    granularity_secs: i64,
) -> Vec<StatsHistoryBucket> {
    let mut bucket_map: HashMap<i64, Vec<&Run>> = HashMap::new();

    for run in runs {
        if run.created_at < start || run.created_at >= end {
            continue;
        }
        if let Some(wf) = workflow_name
            && run.workflow_name != *wf
        {
            continue;
        }
        let bucket_ts = (run.created_at.timestamp() / granularity_secs) * granularity_secs;
        bucket_map.entry(bucket_ts).or_default().push(run);
    }

    let mut buckets: Vec<StatsHistoryBucket> = bucket_map
        .into_iter()
        .map(|(ts, runs)| compute_bucket(ts, &runs, end))
        .collect();
    buckets.sort_by_key(|b| b.time);
    buckets
}

fn compute_bucket(ts: i64, runs: &[&Run], fallback: DateTime<Utc>) -> StatsHistoryBucket {
    let time = DateTime::from_timestamp(ts, 0).unwrap_or(fallback);
    let mut completed = 0u64;
    let mut failed = 0u64;
    let mut cancelled = 0u64;
    let mut total_cost = Decimal::ZERO;
    let mut durations: Vec<u64> = Vec::new();

    for run in runs {
        let has_terminal_duration = matches!(
            run.status.state,
            RunStatus::Completed | RunStatus::Warning | RunStatus::Failed
        );
        match run.status.state {
            RunStatus::Completed | RunStatus::Warning => completed += 1,
            RunStatus::Failed => failed += 1,
            RunStatus::Cancelled => cancelled += 1,
            _ => {}
        }
        if has_terminal_duration && run.duration_ms > 0 {
            durations.push(run.duration_ms);
        }
        total_cost += run.cost_usd;
    }

    let avg_duration_ms = if durations.is_empty() {
        0
    } else {
        durations.iter().sum::<u64>() / durations.len() as u64
    };

    let p95_duration_ms = if durations.is_empty() {
        0
    } else {
        durations.sort_unstable();
        let idx = ((durations.len() as f64 * 0.95).ceil() as usize)
            .min(durations.len())
            .saturating_sub(1);
        durations[idx]
    };

    StatsHistoryBucket {
        time,
        completed,
        failed,
        cancelled,
        avg_duration_ms,
        p95_duration_ms,
        total_cost_usd: total_cost,
    }
}
