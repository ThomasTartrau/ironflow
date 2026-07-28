#![cfg(feature = "store-postgres")]

//! Integration tests for worker leases on PostgreSQL.
//!
//! These tests need a real database (`DATABASE_URL`) because the guarantee they
//! check — two workers never hold the lease on the same run — lives in
//! `FOR UPDATE SKIP LOCKED`, which the in-memory store cannot reproduce.
//!
//! Run them with:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store --features store-postgres --test postgres_lease -- --ignored
//! ```

use std::collections::{HashMap, HashSet};
use std::env::var;
use std::time::Duration;

use ironflow_store::entities::{LeaseRequest, NewRun, RunStatus, TriggerKind};
use ironflow_store::error::StoreError;
use ironflow_store::postgres::PostgresStore;
use ironflow_store::store::{LEASE_EXPIRED_ERROR, RunStore};
use serde_json::json;
use sqlx::{PgPool, query};
use tokio::task::JoinSet;
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run(name: &str, max_retries: u32) -> NewRun {
    NewRun {
        workflow_name: name.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        max_cost_usd: None,
    }
}

fn lease(worker_id: &str, ttl_secs: u64) -> LeaseRequest {
    LeaseRequest {
        worker_id: worker_id.to_string(),
        ttl: Duration::from_secs(ttl_secs),
    }
}

/// Drain every pending run so a test only sees the runs it created.
///
/// The shared test database keeps rows from previous runs; picking without a
/// lease empties the queue without making those runs reapable.
async fn drain_pending(store: &PostgresStore) {
    while store.pick_next_pending(None).await.unwrap().is_some() {}
}

/// Force a run's lease into the past, as if its worker had died.
///
/// `PostgresStore` does not expose its pool, so the test opens its own
/// connection to reach behind the store.
async fn expire_lease(run_id: Uuid) {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPool::connect(&url).await.expect("connect");
    query("UPDATE ironflow.runs SET lease_expires_at = NOW() - interval '1 second' WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("expire lease");
}

#[tokio::test]
#[ignore]
async fn pick_next_pending_attaches_lease() {
    let store = get_store().await;
    drain_pending(&store).await;
    store.create_run(new_run("lease-attach", 3)).await.unwrap();

    let picked = store
        .pick_next_pending(Some(lease("worker-1", 90)))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(picked.status.state, RunStatus::Running);
    assert_eq!(picked.worker_id.as_deref(), Some("worker-1"));
    assert!(picked.lease_expires_at.is_some());
}

#[tokio::test]
#[ignore]
async fn concurrent_workers_never_share_a_lease() {
    let store = get_store().await;
    drain_pending(&store).await;

    const WORKERS: usize = 8;
    for _ in 0..WORKERS {
        store
            .create_run(new_run("lease-concurrency", 3))
            .await
            .unwrap();
    }

    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    let mut set = JoinSet::new();
    for i in 0..WORKERS {
        let url = url.clone();
        set.spawn(async move {
            let store = PostgresStore::new(&url).await.expect("connect");
            store
                .pick_next_pending(Some(lease(&format!("worker-{i}"), 90)))
                .await
                .expect("pick")
        });
    }

    let mut picked = Vec::new();
    while let Some(result) = set.join_next().await {
        if let Some(run) = result.expect("task panicked") {
            picked.push(run);
        }
    }

    assert_eq!(picked.len(), WORKERS, "every run should have been picked");

    let ids: HashSet<Uuid> = picked.iter().map(|r| r.id).collect();
    assert_eq!(ids.len(), WORKERS, "a run was picked twice");

    let owners: HashSet<String> = picked
        .iter()
        .map(|r| r.worker_id.clone().expect("lease set"))
        .collect();
    assert_eq!(owners.len(), WORKERS, "two runs share the same owner");
}

#[tokio::test]
#[ignore]
async fn concurrent_renew_only_succeeds_for_the_owner() {
    let store = get_store().await;
    drain_pending(&store).await;
    store.create_run(new_run("lease-renew", 3)).await.unwrap();
    let picked = store
        .pick_next_pending(Some(lease("worker-1", 90)))
        .await
        .unwrap()
        .unwrap();

    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    let mut set = JoinSet::new();
    for worker in ["worker-1", "worker-2"] {
        let url = url.clone();
        let run_id = picked.id;
        set.spawn(async move {
            let store = PostgresStore::new(&url).await.expect("connect");
            (worker, store.renew_lease(run_id, lease(worker, 90)).await)
        });
    }

    while let Some(result) = set.join_next().await {
        let (worker, outcome) = result.expect("task panicked");
        match worker {
            "worker-1" => assert!(outcome.is_ok(), "owner should renew: {outcome:?}"),
            _ => assert!(
                matches!(outcome, Err(StoreError::LeaseLost { .. })),
                "non-owner should be rejected: {outcome:?}"
            ),
        }
    }
}

#[tokio::test]
#[ignore]
async fn renew_lease_on_unknown_run_is_not_found() {
    let store = get_store().await;

    let err = store
        .renew_lease(Uuid::now_v7(), lease("worker-1", 90))
        .await
        .unwrap_err();

    assert!(matches!(err, StoreError::RunNotFound(_)));
}

#[tokio::test]
#[ignore]
async fn expired_lease_is_requeued_and_picked_by_another_worker() {
    let store = get_store().await;
    drain_pending(&store).await;
    store
        .create_run(new_run("lease-recovery", 3))
        .await
        .unwrap();
    let picked = store
        .pick_next_pending(Some(lease("worker-a", 90)))
        .await
        .unwrap()
        .unwrap();

    expire_lease(picked.id).await;

    let reaped = store.reap_expired_leases(100).await.unwrap();
    assert_eq!(reaped.len(), 1);
    assert_eq!(reaped[0].run.id, picked.id);
    assert_eq!(reaped[0].from, RunStatus::Running);
    assert_eq!(reaped[0].to, RunStatus::Pending);

    let requeued = store.get_run(picked.id).await.unwrap().unwrap();
    assert_eq!(requeued.status.state, RunStatus::Pending);
    assert_eq!(requeued.retry_count, 1);
    assert!(requeued.worker_id.is_none());
    assert!(requeued.lease_expires_at.is_none());

    // The whole point: another worker can now take it over.
    let repicked = store
        .pick_next_pending(Some(lease("worker-b", 90)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repicked.id, picked.id);
    assert_eq!(repicked.worker_id.as_deref(), Some("worker-b"));
}

#[tokio::test]
#[ignore]
async fn reaper_leaves_valid_lease_alone() {
    let store = get_store().await;
    drain_pending(&store).await;
    store.create_run(new_run("lease-valid", 3)).await.unwrap();
    let picked = store
        .pick_next_pending(Some(lease("worker-1", 90)))
        .await
        .unwrap()
        .unwrap();

    let reaped = store.reap_expired_leases(100).await.unwrap();
    assert!(!reaped.iter().any(|r| r.run.id == picked.id));

    let after = store.get_run(picked.id).await.unwrap().unwrap();
    assert_eq!(after.status.state, RunStatus::Running);
    assert_eq!(after.retry_count, 0);
}

#[tokio::test]
#[ignore]
async fn reaper_fails_run_once_retries_are_exhausted() {
    let store = get_store().await;
    drain_pending(&store).await;
    store
        .create_run(new_run("lease-exhausted", 0))
        .await
        .unwrap();
    let picked = store
        .pick_next_pending(Some(lease("worker-1", 90)))
        .await
        .unwrap()
        .unwrap();

    expire_lease(picked.id).await;
    let reaped = store.reap_expired_leases(100).await.unwrap();

    assert_eq!(reaped[0].to, RunStatus::Failed);
    let after = store.get_run(picked.id).await.unwrap().unwrap();
    assert_eq!(after.status.state, RunStatus::Failed);
    assert_eq!(after.error.as_deref(), Some(LEASE_EXPIRED_ERROR));
    assert!(after.completed_at.is_some());
}

#[tokio::test]
#[ignore]
async fn concurrent_reapers_never_recover_the_same_run_twice() {
    let store = get_store().await;
    drain_pending(&store).await;

    const RUNS: usize = 6;
    let mut ids = Vec::new();
    for _ in 0..RUNS {
        store.create_run(new_run("lease-reapers", 3)).await.unwrap();
        let picked = store
            .pick_next_pending(Some(lease("worker-dead", 90)))
            .await
            .unwrap()
            .unwrap();
        expire_lease(picked.id).await;
        ids.push(picked.id);
    }

    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    let mut set = JoinSet::new();
    for _ in 0..3 {
        let url = url.clone();
        set.spawn(async move {
            let store = PostgresStore::new(&url).await.expect("connect");
            store.reap_expired_leases(100).await.expect("reap")
        });
    }

    let mut recovered = Vec::new();
    while let Some(result) = set.join_next().await {
        recovered.extend(result.expect("task panicked").into_iter().map(|r| r.run.id));
    }

    let unique: HashSet<Uuid> = recovered.iter().copied().collect();
    assert_eq!(
        unique.len(),
        recovered.len(),
        "a run was recovered by two reapers"
    );
    for id in &ids {
        assert!(unique.contains(id), "run {id} was never recovered");
        let after = store.get_run(*id).await.unwrap().unwrap();
        assert_eq!(after.retry_count, 1, "run {id} was counted twice");
    }
}

#[tokio::test]
#[ignore]
async fn reap_expired_leases_respects_limit() {
    let store = get_store().await;
    drain_pending(&store).await;

    for _ in 0..3 {
        store.create_run(new_run("lease-limit", 3)).await.unwrap();
        let picked = store
            .pick_next_pending(Some(lease("worker-dead", 90)))
            .await
            .unwrap()
            .unwrap();
        expire_lease(picked.id).await;
    }

    assert_eq!(store.reap_expired_leases(2).await.unwrap().len(), 2);
    assert_eq!(store.reap_expired_leases(100).await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore]
async fn terminal_transition_clears_the_lease() {
    let store = get_store().await;
    drain_pending(&store).await;
    store.create_run(new_run("lease-clear", 3)).await.unwrap();
    let picked = store
        .pick_next_pending(Some(lease("worker-1", 90)))
        .await
        .unwrap()
        .unwrap();

    store
        .update_run_status(picked.id, RunStatus::Cancelled)
        .await
        .unwrap();

    let after = store.get_run(picked.id).await.unwrap().unwrap();
    assert!(after.worker_id.is_none());
    assert!(after.lease_expires_at.is_none());
}
