#![cfg(feature = "store-postgres")]

//! Integration tests for idempotency-key handling on the PostgreSQL store.
//!
//! These exercise the SQL paths the in-memory store cannot cover: the partial
//! unique index, `ON CONFLICT DO NOTHING`, and the release of a key that
//! outlived [`IDEMPOTENCY_WINDOW`].
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_idempotency -- --ignored
//! ```

use std::collections::HashMap;

use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set")
}

async fn get_store() -> PostgresStore {
    PostgresStore::new(&database_url())
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A key unique to this test run: the database is shared across tests.
fn unique_key(label: &str) -> String {
    format!("test:{label}:{}", Uuid::now_v7())
}

fn new_run(name: &str, key: Option<String>) -> NewRun {
    NewRun {
        workflow_name: name.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 3,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        idempotency_key: key,
    }
}

/// Backdate a run so its key falls outside the retention window.
async fn backdate_run(run_id: Uuid, hours: i64) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to PostgreSQL");

    sqlx::query(
        "UPDATE ironflow.runs SET created_at = created_at - make_interval(hours => $2) WHERE id = $1",
    )
    .bind(run_id)
    .bind(hours as i32)
    .execute(&pool)
    .await
    .expect("failed to backdate run");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn create_run_without_key_never_deduplicates() {
    let store = get_store().await;

    let first = store.create_run(new_run("deploy", None)).await.unwrap();
    let second = store.create_run(new_run("deploy", None)).await.unwrap();

    assert!(first.is_created());
    assert!(second.is_created());
    assert_ne!(first.run().id, second.run().id);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn create_run_persists_and_reloads_the_key() {
    let store = get_store().await;
    let key = unique_key("persist");

    let created = store
        .create_run(new_run("deploy", Some(key.clone())))
        .await
        .unwrap()
        .into_run();

    let reloaded = store.get_run(created.id).await.unwrap().unwrap();

    assert_eq!(reloaded.idempotency_key.as_deref(), Some(key.as_str()));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn create_run_replays_a_known_key() {
    let store = get_store().await;
    let key = unique_key("replay");

    let first = store
        .create_run(new_run("deploy", Some(key.clone())))
        .await
        .unwrap();
    let second = store
        .create_run(new_run("deploy", Some(key)))
        .await
        .unwrap();

    assert!(first.is_created());
    assert!(!second.is_created());
    assert_eq!(first.run().id, second.run().id);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn create_run_replays_across_different_workflows() {
    let store = get_store().await;
    let key = unique_key("global");

    let first = store
        .create_run(new_run("deploy", Some(key.clone())))
        .await
        .unwrap();
    let second = store
        .create_run(new_run("rollback", Some(key)))
        .await
        .unwrap();

    // The unique index covers the key alone, not (workflow, key).
    assert!(!second.is_created());
    assert_eq!(second.run().workflow_name, "deploy");
    assert_eq!(first.run().id, second.run().id);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn find_run_by_idempotency_key_returns_the_bound_run() {
    let store = get_store().await;
    let key = unique_key("lookup");

    let created = store
        .create_run(new_run("deploy", Some(key.clone())))
        .await
        .unwrap()
        .into_run();

    let found = store.find_run_by_idempotency_key(&key).await.unwrap();

    assert_eq!(found.expect("run bound to the key").id, created.id);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn find_run_by_idempotency_key_returns_none_for_unknown_key() {
    let store = get_store().await;

    let found = store
        .find_run_by_idempotency_key(&unique_key("absent"))
        .await
        .unwrap();

    assert!(found.is_none());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn expired_key_is_not_returned_by_lookup() {
    let store = get_store().await;
    let key = unique_key("expired-lookup");

    let created = store
        .create_run(new_run("deploy", Some(key.clone())))
        .await
        .unwrap()
        .into_run();
    backdate_run(created.id, 25).await;

    let found = store.find_run_by_idempotency_key(&key).await.unwrap();

    assert!(found.is_none());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn expired_key_is_released_and_reused() {
    let store = get_store().await;
    let key = unique_key("expired-reuse");

    let first = store
        .create_run(new_run("deploy", Some(key.clone())))
        .await
        .unwrap()
        .into_run();
    backdate_run(first.id, 25).await;

    let second = store
        .create_run(new_run("deploy", Some(key.clone())))
        .await
        .unwrap();

    assert!(
        second.is_created(),
        "the stale key must not block insertion"
    );
    assert_ne!(second.run().id, first.id);

    // The stale run lost the key; the fresh one holds it.
    let stale = store.get_run(first.id).await.unwrap().unwrap();
    assert!(stale.idempotency_key.is_none());
    assert_eq!(
        store
            .find_run_by_idempotency_key(&key)
            .await
            .unwrap()
            .expect("run bound to the key")
            .id,
        second.run().id
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn concurrent_creates_with_the_same_key_produce_one_run() {
    let store = get_store().await;
    let key = unique_key("race");
    let mut handles = Vec::new();

    for _ in 0..20 {
        let store = store.clone();
        let key = key.clone();
        handles.push(tokio::spawn(async move {
            store.create_run(new_run("deploy", Some(key))).await
        }));
    }

    let mut created = 0;
    let mut ids = std::collections::HashSet::new();
    for handle in handles {
        let creation = handle
            .await
            .expect("task panicked")
            .expect("create_run must not surface a unique-violation error");
        if creation.is_created() {
            created += 1;
        }
        ids.insert(creation.run().id);
    }

    assert_eq!(created, 1, "exactly one caller should create the run");
    assert_eq!(ids.len(), 1, "all callers should resolve to the same run");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn concurrent_creates_with_distinct_keys_produce_distinct_runs() {
    let store = get_store().await;
    let prefix = unique_key("distinct");
    let mut handles = Vec::new();

    for i in 0..10 {
        let store = store.clone();
        let key = format!("{prefix}:{i}");
        handles.push(tokio::spawn(async move {
            store
                .create_run(new_run("deploy", Some(key)))
                .await
                .unwrap()
        }));
    }

    let mut ids = std::collections::HashSet::new();
    for handle in handles {
        let creation = handle.await.unwrap();
        assert!(creation.is_created());
        ids.insert(creation.run().id);
    }

    assert_eq!(ids.len(), 10);
}
