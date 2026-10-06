#![cfg(feature = "store-postgres")]

//! Integration tests for `list_active_descendants` on the PostgreSQL store.
//!
//! The lookup is a recursive query over the parent label: only a real
//! database exercises it. They need a live database and are ignored by
//! default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_descendants -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use ironflow_store::entities::PARENT_RUN_ID_LABEL;
use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use serde_json::{json, to_value};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn database_url() -> String {
    var("DATABASE_URL").expect("DATABASE_URL must be set")
}

async fn get_store() -> PostgresStore {
    PostgresStore::new(&database_url())
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run(trigger: TriggerKind, labels: HashMap<String, String>) -> NewRun {
    NewRun {
        workflow_name: "descendants".to_string(),
        trigger,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels,
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        priority: 0,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
    }
}

async fn create_root(store: &PostgresStore) -> Run {
    store
        .create_run(new_run(TriggerKind::Manual, HashMap::new()))
        .await
        .unwrap()
        .into_run()
}

/// Create a sub-workflow child of `parent`, moved to `status`.
async fn create_child(store: &PostgresStore, parent: Uuid, status: RunStatus) -> Run {
    let labels = HashMap::from([(PARENT_RUN_ID_LABEL.to_string(), parent.to_string())]);
    let child = store
        .create_run(new_run(TriggerKind::Workflow, labels))
        .await
        .unwrap()
        .into_run();
    if status != RunStatus::Pending {
        store
            .update_run_status(child.id, RunStatus::Running)
            .await
            .unwrap();
    }
    if !matches!(status, RunStatus::Pending | RunStatus::Running) {
        store.update_run_status(child.id, status).await.unwrap();
    }
    child
}

fn ids(runs: &[Run]) -> Vec<Uuid> {
    runs.iter().map(|r| r.id).collect()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn active_descendants_of_a_run_without_children_is_empty() {
    let store = get_store().await;
    let root = create_root(&store).await;

    assert!(
        store
            .list_active_descendants(root.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_active_descendants(Uuid::now_v7())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn active_descendants_are_found_at_any_depth_oldest_first() {
    let store = get_store().await;
    let root = create_root(&store).await;
    let child = create_child(&store, root.id, RunStatus::Running).await;
    let grandchild = create_child(&store, child.id, RunStatus::AwaitingApproval).await;
    let sibling = create_child(&store, root.id, RunStatus::Pending).await;

    let found = store.list_active_descendants(root.id).await.unwrap();
    assert_eq!(ids(&found), [child.id, grandchild.id, sibling.id]);
    assert_eq!(found[1].status.state, RunStatus::AwaitingApproval);

    let below_child = store.list_active_descendants(child.id).await.unwrap();
    assert_eq!(ids(&below_child), [grandchild.id]);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn active_descendants_skip_terminal_runs_but_cross_them() {
    let store = get_store().await;
    let root = create_root(&store).await;
    let finished = create_child(&store, root.id, RunStatus::Completed).await;
    let left_running = create_child(&store, finished.id, RunStatus::Running).await;
    create_child(&store, root.id, RunStatus::Cancelled).await;

    let found = store.list_active_descendants(root.id).await.unwrap();
    assert_eq!(ids(&found), [left_running.id]);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn active_descendants_ignore_runs_not_started_by_a_workflow_step() {
    let store = get_store().await;
    let root = create_root(&store).await;
    let labels = HashMap::from([(PARENT_RUN_ID_LABEL.to_string(), root.id.to_string())]);
    store
        .create_run(new_run(TriggerKind::Manual, labels))
        .await
        .unwrap();

    assert!(
        store
            .list_active_descendants(root.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn active_descendants_stop_on_a_label_cycle() {
    let store = get_store().await;
    let root = create_root(&store).await;
    let child = create_child(&store, root.id, RunStatus::Running).await;
    let grandchild = create_child(&store, child.id, RunStatus::Running).await;

    // The root claims to be a child of its own grand-child. No store method
    // rewrites labels, so the row is edited directly.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to PostgreSQL");
    let labels = HashMap::from([(PARENT_RUN_ID_LABEL.to_string(), grandchild.id.to_string())]);
    sqlx::query("UPDATE ironflow.runs SET labels = $2, trigger = $3 WHERE id = $1")
        .bind(root.id)
        .bind(to_value(&labels).unwrap())
        .bind(to_value(&TriggerKind::Workflow).unwrap())
        .execute(&pool)
        .await
        .unwrap();

    let found = store.list_active_descendants(root.id).await.unwrap();
    assert_eq!(ids(&found), [child.id, grandchild.id]);
}
