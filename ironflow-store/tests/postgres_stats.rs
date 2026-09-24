#![cfg(all(feature = "store-postgres", feature = "store-memory"))]

//! Parity tests for run statistics between PostgreSQL and the in-memory store.
//!
//! Builds the same set of runs (one per status) in both stores and checks that
//! `get_stats` and `get_stats_history` agree. Every run of a test carries a
//! unique `run_id` label, and every query filters on it, so tests are isolated
//! from other data in the database. Requires a live database:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_stats -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use chrono::{Datelike, Timelike, Weekday};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use serde_json::json;
use uuid::Uuid;

const ALL_STATUSES: [RunStatus; 9] = [
    RunStatus::Pending,
    RunStatus::Running,
    RunStatus::Retrying,
    RunStatus::AwaitingApproval,
    RunStatus::Sleeping,
    RunStatus::Completed,
    RunStatus::Warning,
    RunStatus::Failed,
    RunStatus::Cancelled,
];

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn isolation_labels(test_id: &str) -> HashMap<String, String> {
    HashMap::from([("run_id".to_string(), test_id.to_string())])
}

fn new_run(labels: HashMap<String, String>) -> NewRun {
    NewRun {
        workflow_name: "stats-parity".to_string(),
        trigger: TriggerKind::Api,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels,
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        max_cost_usd: None,
    }
}

/// Create one run per status, driven through valid FSM transitions.
async fn seed_one_run_per_status(store: &dyn RunStore, labels: &HashMap<String, String>) {
    for status in ALL_STATUSES {
        let run = store
            .create_run(new_run(labels.clone()))
            .await
            .expect("create run")
            .into_run();
        let path: &[RunStatus] = match status {
            RunStatus::Pending => &[],
            RunStatus::Running => &[RunStatus::Running],
            RunStatus::Retrying => &[RunStatus::Running, RunStatus::Retrying],
            RunStatus::AwaitingApproval => &[RunStatus::Running, RunStatus::AwaitingApproval],
            RunStatus::Sleeping => &[RunStatus::Running, RunStatus::Sleeping],
            RunStatus::Completed => &[RunStatus::Running, RunStatus::Completed],
            RunStatus::Warning => &[RunStatus::Running, RunStatus::Warning],
            RunStatus::Failed => &[RunStatus::Running, RunStatus::Failed],
            RunStatus::Cancelled => &[RunStatus::Cancelled],
        };
        for next in path {
            store
                .update_run_status(run.id, *next)
                .await
                .expect("valid transition");
        }
    }
}

/// Per-status totals over every bucket, in [`ALL_STATUSES`] order.
fn history_totals(buckets: &[StatsHistoryBucket]) -> [u64; 9] {
    let mut totals = [0u64; 9];
    for b in buckets {
        let counts = [
            b.pending,
            b.running,
            b.retrying,
            b.awaiting_approval,
            b.sleeping,
            b.completed,
            b.warning,
            b.failed,
            b.cancelled,
        ];
        for (total, count) in totals.iter_mut().zip(counts) {
            *total += count;
        }
    }
    totals
}

#[tokio::test]
#[ignore]
async fn get_stats_active_runs_match_in_memory() {
    let pg = get_store().await;
    let memory = InMemoryStore::new();
    let labels = isolation_labels(&Uuid::now_v7().to_string());
    seed_one_run_per_status(&pg, &labels).await;
    seed_one_run_per_status(&memory, &labels).await;

    let filter = RunFilter {
        labels: Some(labels.clone()),
        ..RunFilter::default()
    };
    let pg_stats = pg.get_stats(filter.clone()).await.expect("pg stats");
    let memory_stats = memory.get_stats(filter).await.expect("memory stats");

    assert_eq!(pg_stats.total_runs, 9);
    assert_eq!(pg_stats.active_runs, 5);
    assert_eq!(pg_stats.awaiting_approval_runs, 1);
    assert_eq!(pg_stats.total_runs, memory_stats.total_runs);
    assert_eq!(pg_stats.active_runs, memory_stats.active_runs);
    assert_eq!(
        pg_stats.awaiting_approval_runs,
        memory_stats.awaiting_approval_runs
    );
    assert_eq!(pg_stats.completed_runs, memory_stats.completed_runs);
    assert_eq!(pg_stats.failed_runs, memory_stats.failed_runs);
    assert_eq!(pg_stats.cancelled_runs, memory_stats.cancelled_runs);
}

#[tokio::test]
#[ignore]
async fn get_stats_history_counters_match_in_memory() {
    let pg = get_store().await;
    let memory = InMemoryStore::new();
    let labels = isolation_labels(&Uuid::now_v7().to_string());
    seed_one_run_per_status(&pg, &labels).await;
    seed_one_run_per_status(&memory, &labels).await;

    let filter = StatsHistoryFilter {
        labels: Some(labels.clone()),
        period: HistoryPeriod::TwentyFourHours,
        granularity: HistoryGranularity::OneHour,
        ..StatsHistoryFilter::default()
    };
    let pg_buckets = pg
        .get_stats_history(filter.clone())
        .await
        .expect("pg history");
    let memory_buckets = memory
        .get_stats_history(filter)
        .await
        .expect("memory history");

    assert_eq!(history_totals(&pg_buckets), [1; 9]);
    assert_eq!(history_totals(&pg_buckets), history_totals(&memory_buckets));
}

#[tokio::test]
#[ignore]
async fn get_stats_history_weekly_buckets_start_on_monday() {
    let pg = get_store().await;
    let labels = isolation_labels(&Uuid::now_v7().to_string());
    seed_one_run_per_status(&pg, &labels).await;

    let filter = StatsHistoryFilter {
        labels: Some(labels),
        period: HistoryPeriod::NinetyDays,
        granularity: HistoryGranularity::OneWeek,
        ..StatsHistoryFilter::default()
    };
    let buckets = pg.get_stats_history(filter).await.expect("pg history");

    assert!(!buckets.is_empty());
    for b in &buckets {
        assert_eq!(b.time.weekday(), Weekday::Mon);
        assert_eq!(b.time.hour(), 0);
        assert_eq!(b.time.minute(), 0);
        assert_eq!(b.time.second(), 0);
        assert_eq!(b.time, HistoryGranularity::OneWeek.bucket_start(b.time));
    }
    assert_eq!(history_totals(&buckets), [1; 9]);
}
