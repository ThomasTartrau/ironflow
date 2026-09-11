//! Integration tests for [`RunStore::get_stats_history`].

use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use rust_decimal::Decimal;
use serde_json::json;

fn new_run(name: &str) -> NewRun {
    NewRun {
        created_by: None,
        workflow_name: name.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: std::collections::HashMap::new(),
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

    let filter = StatsHistoryFilter {
        workflow_name: None,
        period: HistoryPeriod::TwentyFourHours,
        granularity: HistoryGranularity::OneHour,
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    assert!(!buckets.is_empty());

    let total_completed: u64 = buckets.iter().map(|b| b.completed).sum();
    let total_failed: u64 = buckets.iter().map(|b| b.failed).sum();
    let total_cancelled: u64 = buckets.iter().map(|b| b.cancelled).sum();

    assert_eq!(total_completed, 1);
    assert_eq!(total_failed, 1);
    assert_eq!(total_cancelled, 1);
}

#[tokio::test]
async fn stats_history_empty_store_returns_empty() {
    let store = InMemoryStore::new();

    let filter = StatsHistoryFilter {
        workflow_name: None,
        period: HistoryPeriod::SevenDays,
        granularity: HistoryGranularity::OneDay,
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
    };
    let buckets = store.get_stats_history(filter).await.unwrap();

    for w in buckets.windows(2) {
        assert!(w[0].time <= w[1].time);
    }
}
