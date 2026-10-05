#![cfg(feature = "store-postgres")]

//! Integration tests for concurrency-key exclusivity on the PostgreSQL store.
//!
//! The run status lives in `lib_fsm`, so exclusivity relies on a transactional
//! advisory lock taken in `create_run`. Only a real database exercises it.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_concurrency_key -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use serde_json::json;
use tokio::spawn;
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A key unique to this test run: the database is shared across tests.
fn unique_key(label: &str) -> String {
    format!("test:{label}:{}", Uuid::now_v7())
}

fn new_run(key: &str) -> NewRun {
    NewRun {
        workflow_name: "deploy".to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: Some(key.to_string()),
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn concurrent_creates_with_the_same_concurrency_key_create_one_run() {
    let store = get_store().await;
    let key = unique_key("race");
    let mut handles = Vec::new();

    for _ in 0..20 {
        let store = store.clone();
        let key = key.clone();
        handles.push(spawn(async move { store.create_run(new_run(&key)).await }));
    }

    let mut created = Vec::new();
    let mut conflicts = Vec::new();
    for handle in handles {
        match handle.await.expect("task panicked") {
            Ok(creation) => {
                assert!(creation.is_created());
                created.push(creation.into_run().id);
            }
            Err(StoreError::ConcurrencyConflict { key: k, run_id }) => {
                assert_eq!(k, key);
                conflicts.push(run_id);
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    assert_eq!(created.len(), 1, "exactly one caller should create the run");
    assert_eq!(conflicts.len(), 19);
    assert!(conflicts.iter().all(|id| *id == created[0]));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn concurrency_key_is_released_when_the_run_completes() {
    let store = get_store().await;
    let key = unique_key("release");

    let first = store.create_run(new_run(&key)).await.unwrap().into_run();
    assert_eq!(first.concurrency_key.as_deref(), Some(key.as_str()));

    match store.create_run(new_run(&key)).await {
        Err(StoreError::ConcurrencyConflict { run_id, .. }) => assert_eq!(run_id, first.id),
        other => panic!("expected a concurrency conflict, got {other:?}"),
    }

    store
        .update_run_status(first.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(first.id, RunStatus::Completed)
        .await
        .unwrap();

    let second = store.create_run(new_run(&key)).await.unwrap();
    assert!(second.is_created());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn concurrency_key_is_kept_while_the_run_awaits_approval() {
    let store = get_store().await;
    let key = unique_key("approval");

    let holder = store.create_run(new_run(&key)).await.unwrap().into_run();
    store
        .update_run_status(holder.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(holder.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();

    match store.create_run(new_run(&key)).await {
        Err(StoreError::ConcurrencyConflict { run_id, .. }) => assert_eq!(run_id, holder.id),
        other => panic!("expected a concurrency conflict, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn idempotent_replay_with_a_concurrency_key_returns_the_existing_run() {
    let store = get_store().await;
    let key = unique_key("replay");
    let req = NewRun {
        idempotency_key: Some(unique_key("idem")),
        ..new_run(&key)
    };

    let first = store.create_run(req.clone()).await.unwrap();
    let replay = store.create_run(req).await.unwrap();

    assert!(first.is_created());
    assert!(!replay.is_created());
    assert_eq!(replay.run().id, first.run().id);
}
