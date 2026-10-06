//! Integration tests for run priority on the in-memory store.
//!
//! `pick_next_pending` serves due runs by priority, highest first, then in
//! creation order. The PostgreSQL counterpart lives in `postgres_priority.rs`.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use ironflow_store::prelude::*;
use serde_json::json;
use tokio::time::sleep;

fn new_run(name: &str, priority: i16) -> NewRun {
    NewRun {
        workflow_name: name.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        priority,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
        worker_tags: Vec::new(),
    }
}

async fn create(store: &InMemoryStore, req: NewRun) -> Run {
    let run = store.create_run(req).await.expect("create run").into_run();
    // Distinct created_at values, so FIFO among equal priorities is observable.
    sleep(Duration::from_millis(2)).await;
    run
}

async fn pick(store: &InMemoryStore) -> Option<Run> {
    store.pick_next_pending(None).await.expect("pick")
}

#[tokio::test]
async fn priority_orders_the_queue_then_fifo() {
    let store = InMemoryStore::new();
    let default_old = create(&store, new_run("default-old", 0)).await;
    let background = create(&store, new_run("background", -10)).await;
    let urgent = create(&store, new_run("urgent", 50)).await;
    let default_young = create(&store, new_run("default-young", 0)).await;

    let order: Vec<_> = [
        pick(&store).await,
        pick(&store).await,
        pick(&store).await,
        pick(&store).await,
    ]
    .into_iter()
    .map(|run| run.expect("a run is due").id)
    .collect();

    assert_eq!(
        order,
        vec![urgent.id, default_old.id, default_young.id, background.id]
    );
    assert!(pick(&store).await.is_none());
}

#[tokio::test]
async fn priority_does_not_bypass_scheduled_at() {
    let store = InMemoryStore::new();
    create(
        &store,
        NewRun {
            scheduled_at: Some(Utc::now() + TimeDelta::seconds(3600)),
            ..new_run("deferred", 100)
        },
    )
    .await;
    let due = create(&store, new_run("due", -100)).await;

    assert_eq!(pick(&store).await.expect("due run").id, due.id);
    assert!(pick(&store).await.is_none());
}

#[tokio::test]
async fn priority_higher_run_held_by_saturated_group_does_not_block_others() {
    let store = InMemoryStore::new();
    let group = vec![ConcurrencyLimit::new("repo:acme", 1)];
    let holder = create(
        &store,
        NewRun {
            concurrency_limits: group.clone(),
            ..new_run("holder", 0)
        },
    )
    .await;
    assert_eq!(pick(&store).await.expect("holder").id, holder.id);

    create(
        &store,
        NewRun {
            concurrency_limits: group,
            ..new_run("blocked-urgent", 90)
        },
    )
    .await;
    let free = create(&store, new_run("free", 0)).await;

    assert_eq!(pick(&store).await.expect("free run").id, free.id);
}

#[tokio::test]
async fn priority_defaults_to_zero_and_round_trips() {
    let store = InMemoryStore::new();
    let run = create(&store, new_run("default", 0)).await;
    assert_eq!(run.priority, 0);

    let urgent = create(&store, new_run("urgent", 7)).await;
    let fetched = store
        .get_run(urgent.id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(fetched.priority, 7);
}

#[tokio::test]
async fn priority_out_of_range_is_rejected() {
    let store = InMemoryStore::new();
    for priority in [MAX_PRIORITY + 1, MIN_PRIORITY - 1] {
        let err = store
            .create_run(new_run("out-of-range", priority))
            .await
            .expect_err("out of range priority");
        assert!(matches!(err, StoreError::Database(_)), "{err:?}");
    }
    assert!(store.create_run(new_run("max", MAX_PRIORITY)).await.is_ok());
    assert!(store.create_run(new_run("min", MIN_PRIORITY)).await.is_ok());
}

#[test]
fn priority_validation_bounds() {
    assert!(validate_priority(MIN_PRIORITY).is_ok());
    assert!(validate_priority(MAX_PRIORITY).is_ok());
    assert!(validate_priority(MAX_PRIORITY + 1).is_err());
    assert!(validate_priority(MIN_PRIORITY - 1).is_err());
}

#[tokio::test]
async fn priority_filter_lists_exact_matches() {
    let store = InMemoryStore::new();
    let urgent = create(&store, new_run("urgent", 10)).await;
    create(&store, new_run("default", 0)).await;

    let page = store
        .list_runs(
            RunFilter {
                priority: Some(10),
                ..RunFilter::default()
            },
            1,
            10,
        )
        .await
        .expect("list");
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].id, urgent.id);
}
