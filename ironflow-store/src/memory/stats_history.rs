//! In-memory aggregation for [`RunStore::get_stats_history`].
//!
//! [`RunStore::get_stats_history`]: crate::store::RunStore::get_stats_history

use std::collections::HashMap;

use chrono::DateTime;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::entities::{HistoryGranularity, Run, RunStatus, StatsHistoryBucket};

/// Aggregate runs into time-bucketed statistics.
///
/// Keeps runs created in `[start, end)`, groups them by the UTC bucket of their
/// creation time (see [`HistoryGranularity::bucket_start`]), and computes
/// per-bucket aggregates. Run filters are applied by the caller.
/// Returns buckets sorted by time ascending; empty periods are omitted
/// (the API fills them).
pub(crate) fn aggregate_history_buckets<'a>(
    runs: impl Iterator<Item = &'a Run>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    granularity: HistoryGranularity,
) -> Vec<StatsHistoryBucket> {
    let mut bucket_map: HashMap<DateTime<Utc>, Vec<&Run>> = HashMap::new();

    for run in runs {
        if run.created_at < start || run.created_at >= end {
            continue;
        }
        bucket_map
            .entry(granularity.bucket_start(run.created_at))
            .or_default()
            .push(run);
    }

    let mut buckets: Vec<StatsHistoryBucket> = bucket_map
        .into_iter()
        .map(|(time, runs)| compute_bucket(time, &runs))
        .collect();
    buckets.sort_by_key(|b| b.time);
    buckets
}

fn compute_bucket(time: DateTime<Utc>, runs: &[&Run]) -> StatsHistoryBucket {
    let mut bucket = StatsHistoryBucket {
        time,
        ..StatsHistoryBucket::default()
    };
    let mut total_cost = Decimal::ZERO;
    let mut durations: Vec<u64> = Vec::new();

    for run in runs {
        let has_terminal_duration = matches!(
            run.status.state,
            RunStatus::Completed | RunStatus::Warning | RunStatus::Failed
        );
        match run.status.state {
            RunStatus::Pending => bucket.pending += 1,
            RunStatus::Running => bucket.running += 1,
            RunStatus::Completed => bucket.completed += 1,
            RunStatus::Failed => bucket.failed += 1,
            RunStatus::Retrying => bucket.retrying += 1,
            RunStatus::Cancelled => bucket.cancelled += 1,
            RunStatus::AwaitingApproval => bucket.awaiting_approval += 1,
            RunStatus::Warning => bucket.warning += 1,
            RunStatus::Sleeping => bucket.sleeping += 1,
        }
        if has_terminal_duration && run.duration_ms > 0 {
            durations.push(run.duration_ms);
        }
        total_cost += run.cost_usd;
    }

    bucket.avg_duration_ms = if durations.is_empty() {
        0
    } else {
        durations.iter().sum::<u64>() / durations.len() as u64
    };

    bucket.p95_duration_ms = if durations.is_empty() {
        0
    } else {
        durations.sort_unstable();
        let idx = ((durations.len() as f64 * 0.95).ceil() as usize)
            .min(durations.len())
            .saturating_sub(1);
        durations[idx]
    };

    bucket.total_cost_usd = total_cost;
    bucket
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Duration, Timelike, Weekday};

    use super::*;
    use crate::memory::InMemoryStore;
    use crate::memory::tests::new_run_req;
    use crate::store::RunStore;

    const ALL_STATUSES: [RunStatus; 9] = [
        RunStatus::Pending,
        RunStatus::Running,
        RunStatus::Completed,
        RunStatus::Failed,
        RunStatus::Retrying,
        RunStatus::Cancelled,
        RunStatus::AwaitingApproval,
        RunStatus::Warning,
        RunStatus::Sleeping,
    ];

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().expect("valid timestamp")
    }

    async fn make_run(store: &InMemoryStore, status: RunStatus, created_at: DateTime<Utc>) -> Run {
        let mut run = store
            .create_run(new_run_req("wf"))
            .await
            .expect("create run")
            .into_run();
        run.status.state = status;
        run.created_at = created_at;
        run
    }

    #[tokio::test]
    async fn each_status_is_counted_in_its_own_field() {
        let store = InMemoryStore::new();
        let created_at = utc("2026-09-24T10:15:00Z");
        let mut runs = Vec::new();
        for status in ALL_STATUSES {
            runs.push(make_run(&store, status, created_at).await);
        }

        let buckets = aggregate_history_buckets(
            runs.iter(),
            utc("2026-09-24T00:00:00Z"),
            utc("2026-09-25T00:00:00Z"),
            HistoryGranularity::OneHour,
        );

        assert_eq!(buckets.len(), 1);
        let b = &buckets[0];
        assert_eq!(b.time, utc("2026-09-24T10:00:00Z"));
        assert_eq!(b.pending, 1);
        assert_eq!(b.running, 1);
        assert_eq!(b.completed, 1);
        assert_eq!(b.failed, 1);
        assert_eq!(b.retrying, 1);
        assert_eq!(b.cancelled, 1);
        assert_eq!(b.awaiting_approval, 1);
        assert_eq!(b.warning, 1);
        assert_eq!(b.sleeping, 1);
    }

    #[tokio::test]
    async fn week_buckets_start_on_monday() {
        let store = InMemoryStore::new();
        let mut runs = Vec::new();
        let mut time = utc("2026-07-01T09:30:00Z");
        for _ in 0..20 {
            runs.push(make_run(&store, RunStatus::Completed, time).await);
            time += Duration::hours(97);
        }

        let buckets = aggregate_history_buckets(
            runs.iter(),
            utc("2026-06-01T00:00:00Z"),
            utc("2026-10-01T00:00:00Z"),
            HistoryGranularity::OneWeek,
        );

        assert!(!buckets.is_empty());
        let total: u64 = buckets.iter().map(|b| b.completed).sum();
        assert_eq!(total, 20);
        for b in &buckets {
            assert_eq!(b.time.weekday(), Weekday::Mon);
            assert_eq!(b.time.hour(), 0);
            assert_eq!(b.time.minute(), 0);
        }
        assert!(buckets.windows(2).all(|w| w[0].time < w[1].time));
    }

    #[tokio::test]
    async fn runs_outside_range_are_ignored() {
        let store = InMemoryStore::new();
        let start = utc("2026-09-24T00:00:00Z");
        let end = utc("2026-09-25T00:00:00Z");
        let runs = [
            make_run(&store, RunStatus::Completed, start - Duration::seconds(1)).await,
            make_run(&store, RunStatus::Completed, start).await,
            make_run(&store, RunStatus::Failed, end - Duration::seconds(1)).await,
            make_run(&store, RunStatus::Failed, end).await,
        ];

        let buckets =
            aggregate_history_buckets(runs.iter(), start, end, HistoryGranularity::OneDay);

        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].time, start);
        assert_eq!(buckets[0].completed, 1);
        assert_eq!(buckets[0].failed, 1);
    }

    #[tokio::test]
    async fn durations_only_cover_terminal_runs() {
        let store = InMemoryStore::new();
        let created_at = utc("2026-09-24T10:15:00Z");
        let mut completed = make_run(&store, RunStatus::Completed, created_at).await;
        completed.duration_ms = 1000;
        let mut warning = make_run(&store, RunStatus::Warning, created_at).await;
        warning.duration_ms = 3000;
        let mut running = make_run(&store, RunStatus::Running, created_at).await;
        running.duration_ms = 100_000;
        let runs = [completed, warning, running];

        let buckets = aggregate_history_buckets(
            runs.iter(),
            utc("2026-09-24T00:00:00Z"),
            utc("2026-09-25T00:00:00Z"),
            HistoryGranularity::OneDay,
        );

        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].avg_duration_ms, 2000);
        assert_eq!(buckets[0].p95_duration_ms, 3000);
    }
}
