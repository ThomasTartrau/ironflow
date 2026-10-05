#![cfg(feature = "store-postgres")]

//! Integration tests for run concurrency groups on the PostgreSQL store.
//!
//! The group gate of `pick_next_pending` relies on per-group advisory locks
//! and a recount under `READ COMMITTED`. Only a real database exercises it.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_concurrency_limits -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use serde_json::json;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

/// Serialises the tests of this file. They pick from the global queue, so two
/// of them running at once would steal each other's runs.
static SERIAL: Mutex<()> = Mutex::const_new(());

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A group unique to this test run: the database is shared across tests.
fn unique_group(label: &str) -> String {
    format!("test:{label}:{}", Uuid::now_v7())
}

fn new_run(limits: &[(&str, u32)]) -> NewRun {
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
        concurrency_key: None,
        concurrency_limits: limits
            .iter()
            .map(|(group, limit)| ConcurrencyLimit::new(*group, *limit))
            .collect(),
        max_cost_usd: None,
    }
}

async fn create(store: &PostgresStore, req: NewRun) -> Run {
    store.create_run(req).await.unwrap().into_run()
}

async fn pick_id(store: &PostgresStore) -> Option<Uuid> {
    store.pick_next_pending(None).await.unwrap().map(|r| r.id)
}

/// Drain every pickable run so a test only sees the runs it created.
async fn drain_pending(store: &PostgresStore) {
    while store.pick_next_pending(None).await.unwrap().is_some() {}
}

/// Move the given runs to a terminal state so they neither hold a group slot
/// nor stay in the queue for the next test.
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
async fn create_run_persists_concurrency_limits() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let group = unique_group("persist");
    let limits = [(group.as_str(), 3), ("tenant:42", 1)];
    let run = create(&store, new_run(&limits)).await;

    let fetched = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(
        fetched.concurrency_limits,
        vec![
            ConcurrencyLimit::new(group.as_str(), 3),
            ConcurrencyLimit::new("tenant:42", 1),
        ]
    );

    finish(&store, &[run.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn create_run_rejects_invalid_concurrency_limits() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let group = unique_group("invalid");

    let err = store
        .create_run(new_run(&[(group.as_str(), 0)]))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StoreError::InvalidConcurrencyLimit(ConcurrencyLimitError::ZeroLimit { .. })
    ));

    let err = store
        .create_run(new_run(&[(group.as_str(), 1), (group.as_str(), 2)]))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StoreError::InvalidConcurrencyLimit(ConcurrencyLimitError::DuplicateGroup { .. })
    ));

    let page = store
        .list_runs(
            RunFilter {
                concurrency_group: Some(group),
                ..RunFilter::default()
            },
            1,
            10,
        )
        .await
        .unwrap();
    assert_eq!(page.total, 0, "an invalid run must not be stored");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_holds_back_run_beyond_group_limit() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    drain_pending(&store).await;
    let group = unique_group("limit");

    let r1 = create(&store, new_run(&[(group.as_str(), 2)])).await;
    let r2 = create(&store, new_run(&[(group.as_str(), 2)])).await;
    let r3 = create(&store, new_run(&[(group.as_str(), 2)])).await;

    assert_eq!(pick_id(&store).await, Some(r1.id));
    assert_eq!(pick_id(&store).await, Some(r2.id));
    assert_eq!(pick_id(&store).await, None, "the group is saturated");

    store
        .update_run_status(r1.id, RunStatus::Completed)
        .await
        .unwrap();
    assert_eq!(pick_id(&store).await, Some(r3.id));

    finish(&store, &[r1.id, r2.id, r3.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_skips_blocked_group_for_other_group() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    drain_pending(&store).await;
    let a = unique_group("a");
    let b = unique_group("b");

    let a1 = create(&store, new_run(&[(a.as_str(), 1)])).await;
    assert_eq!(pick_id(&store).await, Some(a1.id));

    let a2 = create(&store, new_run(&[(a.as_str(), 1)])).await;
    let b1 = create(&store, new_run(&[(b.as_str(), 1)])).await;
    let free = create(&store, new_run(&[])).await;

    assert_eq!(pick_id(&store).await, Some(b1.id));
    assert_eq!(pick_id(&store).await, Some(free.id));
    assert_eq!(pick_id(&store).await, None);

    let held = store.get_run(a2.id).await.unwrap().unwrap();
    assert_eq!(held.status.state, RunStatus::Pending);

    let blocked = store.count_blocked_runs_by_group().await.unwrap();
    let backlog = blocked
        .iter()
        .find(|g| g.group == a)
        .expect("group a has a blocked run");
    assert_eq!(backlog.blocked_runs, 1);
    assert!(blocked.iter().all(|g| g.group != b));

    finish(&store, &[a1.id, a2.id, b1.id, free.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_does_not_count_sub_workflow_runs() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    drain_pending(&store).await;
    let group = unique_group("child");

    // A sub-workflow run carrying the group never takes a slot of its own.
    let child = create(
        &store,
        NewRun {
            trigger: TriggerKind::Workflow,
            ..new_run(&[(group.as_str(), 1)])
        },
    )
    .await;
    assert_eq!(pick_id(&store).await, Some(child.id));

    let root = create(&store, new_run(&[(group.as_str(), 1)])).await;
    assert_eq!(pick_id(&store).await, Some(root.id));

    finish(&store, &[child.id, root.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn pick_next_pending_sleeping_run_frees_group_slot() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    drain_pending(&store).await;
    let group = unique_group("sleep");

    let a = create(&store, new_run(&[(group.as_str(), 1)])).await;
    let b = create(&store, new_run(&[(group.as_str(), 1)])).await;
    assert_eq!(pick_id(&store).await, Some(a.id));
    assert_eq!(pick_id(&store).await, None, "the group is saturated");

    // A sleeping run is not running: it gives its slot back.
    store
        .update_run_status(a.id, RunStatus::Sleeping)
        .await
        .unwrap();
    assert_eq!(pick_id(&store).await, Some(b.id));

    finish(&store, &[a.id, b.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn concurrent_picks_on_single_slot_group_only_one_wins() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    drain_pending(&store).await;
    let group = unique_group("race");
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");

    let mut ids = Vec::new();
    for _ in 0..10 {
        ids.push(create(&store, new_run(&[(group.as_str(), 1)])).await.id);
    }

    let mut set = JoinSet::new();
    for _ in 0..20 {
        let url = url.clone();
        set.spawn(async move {
            let store = PostgresStore::new(&url).await.expect("connect");
            store.pick_next_pending(None).await.expect("pick")
        });
    }

    let mut picked = Vec::new();
    while let Some(result) = set.join_next().await {
        if let Some(run) = result.expect("task panicked") {
            picked.push(run.id);
        }
    }
    assert_eq!(
        picked.len(),
        1,
        "a single-slot group admits exactly one run"
    );

    let running = store
        .list_runs(
            RunFilter {
                status: Some(RunStatus::Running),
                concurrency_group: Some(group.clone()),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();
    assert_eq!(running.total, 1);

    finish(&store, &ids).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn list_runs_filters_by_concurrency_group() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let group = unique_group("filter");
    let other = unique_group("filter-other");

    let limits = [(group.as_str(), 1), (other.as_str(), 4)];
    let in_group = create(&store, new_run(&limits)).await;
    let outside = create(&store, new_run(&[(other.as_str(), 4)])).await;

    let page = store
        .list_runs(
            RunFilter {
                concurrency_group: Some(group),
                ..RunFilter::default()
            },
            1,
            10,
        )
        .await
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].id, in_group.id);

    finish(&store, &[in_group.id, outside.id]).await;
}
