#![cfg(feature = "store-postgres")]

//! Integration tests for worker routing on the PostgreSQL store.
//!
//! The workflow and tag filters of `pick_next_pending_for` run in the candidate
//! SELECT, before `FOR UPDATE SKIP LOCKED`. Only a real database exercises it.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_worker_routing -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Serialises the tests of this file. The legacy pick takes from the global
/// queue, so two tests running at once could steal each other's runs.
static SERIAL: Mutex<()> = Mutex::const_new(());

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A workflow name unique to this test run: the database is shared across tests.
fn unique_workflow(label: &str) -> String {
    format!("routing:{label}:{}", Uuid::now_v7())
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}

fn new_run(workflow: &str, tags: &[&str]) -> NewRun {
    NewRun {
        workflow_name: workflow.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
        worker_tags: strings(tags),
    }
}

fn caps(workflows: &[&str], tags: &[&str]) -> WorkerCapabilities {
    WorkerCapabilities::new(Some(strings(workflows)), strings(tags))
}

async fn create(store: &PostgresStore, req: NewRun) -> Run {
    store.create_run(req).await.unwrap().into_run()
}

async fn pick_for(store: &PostgresStore, worker: Option<WorkerCapabilities>) -> Option<Uuid> {
    let picked = store.pick_next_pending_for(None, worker).await.unwrap();
    picked.map(|run| run.id)
}

/// Drain every pickable run so a test only sees the runs it created.
async fn drain_pending(store: &PostgresStore) {
    while store.pick_next_pending(None).await.unwrap().is_some() {}
}

/// Move the given runs to a terminal state so they do not stay in the queue
/// for the next test.
async fn finish(store: &PostgresStore, ids: &[Uuid]) {
    for id in ids {
        let run = store.get_run(*id).await.unwrap().unwrap();
        let target = match run.status.state {
            RunStatus::Pending | RunStatus::Sleeping => RunStatus::Cancelled,
            RunStatus::Running => RunStatus::Completed,
            _ => continue,
        };
        store.update_run_status(*id, target).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn create_run_persists_worker_tags_sorted() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let wf = unique_workflow("persist");
    let run = create(&store, new_run(&wf, &["region:eu", "gpu", "gpu"])).await;

    let expected = strings(&["gpu", "region:eu"]);
    assert_eq!(run.worker_tags, expected);
    let fetched = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(fetched.worker_tags, expected);

    finish(&store, &[run.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn create_run_rejects_invalid_worker_tag() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let wf = unique_workflow("invalid");

    let result = store.create_run(new_run(&wf, &["bad,tag"])).await;
    let Err(StoreError::InvalidWorkerTag(err)) = result else {
        panic!("expected an invalid worker tag error");
    };
    assert!(matches!(err, WorkerTagError::InvalidChar { .. }));

    let filter = RunFilter {
        workflow_name: Some(wf),
        ..RunFilter::default()
    };
    let page = store.list_runs(filter, 1, 10).await.unwrap();
    assert!(page.items.is_empty());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_skips_run_whose_tags_worker_lacks() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let wf = unique_workflow("tags");
    let gpu = create(&store, new_run(&wf, &["gpu"])).await;
    let plain = create(&store, new_run(&wf, &[])).await;

    let picked = pick_for(&store, Some(caps(&[&wf], &["arm"]))).await;
    assert_eq!(picked, Some(plain.id));

    let gpu_run = store.get_run(gpu.id).await.unwrap().unwrap();
    assert_eq!(gpu_run.status.state, RunStatus::Pending);

    finish(&store, &[gpu.id, plain.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_skips_unknown_workflow() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let other_wf = unique_workflow("other");
    let known_wf = unique_workflow("known");
    let other = create(&store, new_run(&other_wf, &[])).await;
    let known = create(&store, new_run(&known_wf, &[])).await;

    let picked = pick_for(&store, Some(caps(&[&known_wf], &[]))).await;
    assert_eq!(picked, Some(known.id));

    let other_run = store.get_run(other.id).await.unwrap().unwrap();
    assert_eq!(other_run.status.state, RunStatus::Pending);

    finish(&store, &[other.id, known.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_ineligible_head_does_not_block_queue() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let wf = unique_workflow("head");
    let head = create(&store, new_run(&wf, &["gpu"])).await;
    let younger = create(&store, new_run(&wf, &[])).await;

    let worker = caps(&[&wf], &[]);
    let picked = pick_for(&store, Some(worker.clone())).await;
    assert_eq!(picked, Some(younger.id));
    let next = pick_for(&store, Some(worker)).await;
    assert_eq!(next, None);

    finish(&store, &[head.id, younger.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_without_capabilities_takes_everything() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    drain_pending(&store).await;
    let wf = unique_workflow("legacy");
    let gpu = create(&store, new_run(&wf, &["gpu"])).await;

    let picked = pick_for(&store, None).await;
    assert_eq!(picked, Some(gpu.id));

    finish(&store, &[gpu.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_worker_with_superset_tags_takes_run() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let wf = unique_workflow("superset");
    let run = create(&store, new_run(&wf, &["gpu", "region:eu"])).await;

    let worker = caps(&[&wf, "other"], &["arm", "gpu", "region:eu"]);
    let picked = pick_for(&store, Some(worker)).await;
    assert_eq!(picked, Some(run.id));

    finish(&store, &[run.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_for_returns_none_when_nothing_eligible() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let wf = unique_workflow("none");
    let gpu = create(&store, new_run(&wf, &["gpu"])).await;

    let picked = pick_for(&store, Some(caps(&[&wf], &[]))).await;
    assert_eq!(picked, None);
    let run = store.get_run(gpu.id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Pending);

    finish(&store, &[gpu.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn get_stats_with_eligible_for_counts_only_takeable_runs() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let wf = unique_workflow("stats");
    let gpu = create(&store, new_run(&wf, &["gpu"])).await;
    let plain = create(&store, new_run(&wf, &[])).await;

    let filter = RunFilter {
        workflow_name: Some(wf.clone()),
        eligible_for: Some(caps(&[&wf], &[])),
        ..RunFilter::default()
    };
    let stats = store.get_stats(filter).await.unwrap();
    assert_eq!(stats.total_runs, 1);

    let filter = RunFilter {
        workflow_name: Some(wf.clone()),
        eligible_for: Some(WorkerCapabilities::new(None, strings(&["gpu"]))),
        ..RunFilter::default()
    };
    let stats = store.get_stats(filter).await.unwrap();
    assert_eq!(stats.total_runs, 2);

    let filter = RunFilter {
        eligible_for: Some(caps(&["missing"], &["gpu"])),
        workflow_name: Some(wf),
        ..RunFilter::default()
    };
    let stats = store.get_stats(filter).await.unwrap();
    assert_eq!(stats.total_runs, 0);

    finish(&store, &[gpu.id, plain.id]).await;
}
