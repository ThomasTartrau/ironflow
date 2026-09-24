//! Integration tests for [`RunStore::get_stats_history`].

use std::collections::HashMap;

use chrono::{Datelike, Timelike, Weekday};
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

fn new_run(name: &str) -> NewRun {
    NewRun {
        created_by: None,
        workflow_name: name.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        idempotency_key: None,
        max_cost_usd: None,
    }
}

async fn create_terminal_run(
    store: &InMemoryStore,
    name: &str,
    status: RunStatus,
    duration_ms: u64,
    cost_usd: Decimal,
) -> Run {
    let run = store.create_run(new_run(name)).await.unwrap().into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    store.update_run_status(run.id, status).await.unwrap();
    store
        .update_run(
            run.id,
            RunUpdate {
                duration_ms: Some(duration_ms),
                cost_usd: Some(cost_usd),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();
    store.get_run(run.id).await.unwrap().unwrap()
}

#[tokio::test]
async fn stats_history_aggregates_counts() {
    let store = InMemoryStore::new();

    create_terminal_run(
        &store,
        "deploy",
        RunStatus::Completed,
        5000,
        Decimal::new(100, 2),
    )
    .await;
    create_terminal_run(
        &store,
        "deploy",
        RunStatus::Failed,
        3000,
        Decimal::new(50, 2),
    )
    .await;
    create_terminal_run(&store, "deploy", RunStatus::Cancelled, 0, Decimal::ZERO).await;
    create_terminal_run(
        &store,
        "deploy",
        RunStatus::Warning,
        4000,
        Decimal::new(20, 2),
    )
    .await;

    let filter = StatsHistoryFilter {
        workflow_name: None,
        period: HistoryPeriod::TwentyFourHours,
        granularity: HistoryGranularity::OneHour,

        ..StatsHistoryFilter::default()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    assert!(!buckets.is_empty());

    let total_completed: u64 = buckets.iter().map(|b| b.completed).sum();
    let total_failed: u64 = buckets.iter().map(|b| b.failed).sum();
    let total_cancelled: u64 = buckets.iter().map(|b| b.cancelled).sum();
    let total_warning: u64 = buckets.iter().map(|b| b.warning).sum();

    assert_eq!(total_completed, 1);
    assert_eq!(total_failed, 1);
    assert_eq!(total_cancelled, 1);
    assert_eq!(total_warning, 1);
}

#[tokio::test]
async fn stats_history_empty_store_returns_empty() {
    let store = InMemoryStore::new();

    let filter = StatsHistoryFilter {
        workflow_name: None,
        period: HistoryPeriod::SevenDays,
        granularity: HistoryGranularity::OneDay,

        ..StatsHistoryFilter::default()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();
    assert!(buckets.is_empty());
}

#[tokio::test]
async fn stats_history_filters_by_workflow() {
    let store = InMemoryStore::new();

    create_terminal_run(
        &store,
        "deploy",
        RunStatus::Completed,
        5000,
        Decimal::new(100, 2),
    )
    .await;
    create_terminal_run(
        &store,
        "build",
        RunStatus::Completed,
        3000,
        Decimal::new(50, 2),
    )
    .await;

    let filter = StatsHistoryFilter {
        workflow_name: Some("deploy".to_string()),
        period: HistoryPeriod::TwentyFourHours,
        granularity: HistoryGranularity::OneHour,

        ..StatsHistoryFilter::default()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    let total_completed: u64 = buckets.iter().map(|b| b.completed).sum();
    assert_eq!(total_completed, 1);
}

#[tokio::test]
async fn stats_history_computes_duration_metrics() {
    let store = InMemoryStore::new();

    for dur in [1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000] {
        create_terminal_run(&store, "deploy", RunStatus::Completed, dur, Decimal::ZERO).await;
    }

    let filter = StatsHistoryFilter {
        workflow_name: None,
        period: HistoryPeriod::TwentyFourHours,
        granularity: HistoryGranularity::OneDay,

        ..StatsHistoryFilter::default()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    assert_eq!(buckets.len(), 1);
    let bucket = &buckets[0];
    assert_eq!(bucket.completed, 10);
    assert_eq!(bucket.avg_duration_ms, 5500);
    assert_eq!(bucket.p95_duration_ms, 10000);
}

#[tokio::test]
async fn stats_history_buckets_are_sorted_by_time() {
    let store = InMemoryStore::new();

    for _ in 0..5 {
        create_terminal_run(&store, "deploy", RunStatus::Completed, 1000, Decimal::ZERO).await;
    }

    let filter = StatsHistoryFilter {
        workflow_name: None,
        period: HistoryPeriod::TwentyFourHours,
        granularity: HistoryGranularity::OneHour,

        ..StatsHistoryFilter::default()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    for w in buckets.windows(2) {
        assert!(w[0].time <= w[1].time);
    }
}

/// Sum one counter over every bucket.
fn sum(buckets: &[StatsHistoryBucket], field: fn(&StatsHistoryBucket) -> u64) -> u64 {
    buckets.iter().map(field).sum()
}

/// Create a run and drive it to `status` through valid FSM transitions.
async fn create_run_in_status(store: &InMemoryStore, req: NewRun, status: RunStatus) -> Run {
    let run = store.create_run(req).await.unwrap().into_run();
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
        store.update_run_status(run.id, *next).await.unwrap();
    }
    store.get_run(run.id).await.unwrap().unwrap()
}

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

fn last_24h() -> StatsHistoryFilter {
    StatsHistoryFilter {
        period: HistoryPeriod::TwentyFourHours,
        granularity: HistoryGranularity::OneHour,
        ..StatsHistoryFilter::default()
    }
}

#[tokio::test]
async fn stats_history_counts_every_status() {
    let store = InMemoryStore::new();
    for status in ALL_STATUSES {
        create_run_in_status(&store, new_run("deploy"), status).await;
    }

    let buckets = store.get_stats_history(last_24h()).await.unwrap();

    assert_eq!(sum(&buckets, |b| b.pending), 1);
    assert_eq!(sum(&buckets, |b| b.running), 1);
    assert_eq!(sum(&buckets, |b| b.retrying), 1);
    assert_eq!(sum(&buckets, |b| b.awaiting_approval), 1);
    assert_eq!(sum(&buckets, |b| b.sleeping), 1);
    assert_eq!(sum(&buckets, |b| b.completed), 1);
    assert_eq!(sum(&buckets, |b| b.warning), 1);
    assert_eq!(sum(&buckets, |b| b.failed), 1);
    assert_eq!(sum(&buckets, |b| b.cancelled), 1);
}

#[tokio::test]
async fn stats_history_running_run_in_24h() {
    let store = InMemoryStore::new();
    create_run_in_status(&store, new_run("deploy"), RunStatus::Running).await;

    let buckets = store.get_stats_history(last_24h()).await.unwrap();

    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].running, 1);
    assert_eq!(buckets[0].completed, 0);
    assert_eq!(buckets[0].success_rate_percent(), None);
}

#[tokio::test]
async fn stats_history_weekly_buckets_start_on_monday() {
    let store = InMemoryStore::new();
    for status in ALL_STATUSES {
        create_run_in_status(&store, new_run("deploy"), status).await;
    }

    let filter = StatsHistoryFilter {
        period: HistoryPeriod::NinetyDays,
        granularity: HistoryGranularity::OneWeek,
        ..StatsHistoryFilter::default()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    assert!(!buckets.is_empty());
    for b in &buckets {
        assert_eq!(b.time.weekday(), Weekday::Mon);
        assert_eq!(b.time.hour(), 0);
        assert_eq!(b.time.minute(), 0);
        assert_eq!(b.time.second(), 0);
    }
}

#[tokio::test]
async fn stats_history_filters_by_status() {
    let store = InMemoryStore::new();
    for status in ALL_STATUSES {
        create_run_in_status(&store, new_run("deploy"), status).await;
    }

    let filter = StatsHistoryFilter {
        status: Some(RunStatus::Failed),
        ..last_24h()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    assert_eq!(sum(&buckets, |b| b.failed), 1);
    assert_eq!(sum(&buckets, |b| b.completed), 0);
    assert_eq!(sum(&buckets, |b| b.running), 0);
    assert_eq!(sum(&buckets, |b| b.pending), 0);
}

#[tokio::test]
async fn stats_history_filters_by_label() {
    let store = InMemoryStore::new();
    let mut prod = new_run("deploy");
    prod.labels = HashMap::from([("env".to_string(), "prod".to_string())]);
    let mut staging = new_run("deploy");
    staging.labels = HashMap::from([("env".to_string(), "staging".to_string())]);
    create_run_in_status(&store, prod, RunStatus::Completed).await;
    create_run_in_status(&store, staging, RunStatus::Completed).await;
    create_run_in_status(&store, new_run("deploy"), RunStatus::Completed).await;

    let filter = StatsHistoryFilter {
        labels: Some(HashMap::from([("env".to_string(), "prod".to_string())])),
        ..last_24h()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    assert_eq!(sum(&buckets, |b| b.completed), 1);
}

#[tokio::test]
async fn stats_history_filters_by_created_by() {
    let store = InMemoryStore::new();
    let alice = Uuid::now_v7();
    let bob = Uuid::now_v7();
    let mut by_alice = new_run("deploy");
    by_alice.created_by = Some(RunActor::User { user_id: alice });
    let mut by_bob = new_run("deploy");
    by_bob.created_by = Some(RunActor::User { user_id: bob });
    create_run_in_status(&store, by_alice, RunStatus::Running).await;
    create_run_in_status(&store, by_bob, RunStatus::Running).await;
    create_run_in_status(&store, new_run("deploy"), RunStatus::Running).await;

    let filter = StatsHistoryFilter {
        created_by_user_id: Some(alice),
        ..last_24h()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    assert_eq!(sum(&buckets, |b| b.running), 1);
}

#[tokio::test]
async fn stats_history_filters_by_has_steps() {
    let store = InMemoryStore::new();
    let with_steps = create_run_in_status(&store, new_run("deploy"), RunStatus::Completed).await;
    store
        .create_step(NewStep {
            run_id: with_steps.id,
            trace_id: step_trace_id(with_steps.id, "build", 0),
            name: "build".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .unwrap();
    create_run_in_status(&store, new_run("deploy"), RunStatus::Completed).await;
    create_run_in_status(&store, new_run("deploy"), RunStatus::Completed).await;

    let with = StatsHistoryFilter {
        has_steps: Some(true),
        ..last_24h()
    };
    let buckets = store.get_stats_history(with).await.unwrap();
    assert_eq!(sum(&buckets, |b| b.completed), 1);

    let without = StatsHistoryFilter {
        has_steps: Some(false),
        ..last_24h()
    };
    let buckets = store.get_stats_history(without).await.unwrap();
    assert_eq!(sum(&buckets, |b| b.completed), 2);
}

#[tokio::test]
async fn stats_history_workflow_filter_is_substring() {
    let store = InMemoryStore::new();
    create_run_in_status(&store, new_run("deploy-prod"), RunStatus::Completed).await;
    create_run_in_status(&store, new_run("Deploy-staging"), RunStatus::Completed).await;
    create_run_in_status(&store, new_run("build"), RunStatus::Completed).await;

    let filter = StatsHistoryFilter {
        workflow_name: Some("deploy".to_string()),
        ..last_24h()
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    assert_eq!(sum(&buckets, |b| b.completed), 2);
}
