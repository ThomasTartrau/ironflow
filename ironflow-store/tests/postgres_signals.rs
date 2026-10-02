#![cfg(feature = "store-postgres")]

//! Integration tests for signals on PostgreSQL.
//!
//! These tests need a real database (`DATABASE_URL`): the guarantees they check
//! (a sleeping run wakes exactly once across several wakers, a delivery records
//! the `signal_received` FSM transition) live in SQL.
//!
//! Run them with:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store --features store-postgres --test postgres_signals -- --ignored
//! ```

use std::collections::{HashMap, HashSet};
use std::env::var;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use ironflow_store::entities::{
    NewRun, NewSignal, NewStep, RunStatus, RunUpdate, SignalStepResolution, Step, StepKind,
    StepStatus, StepUpdate, TriggerKind, step_trace_id,
};
use ironflow_store::postgres::{PoolConfig, PostgresStore};
use ironflow_store::signal_store::SignalStore;
use ironflow_store::store::RunStore;
use serde_json::json;
use sqlx::{PgPool, query_scalar};
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

/// Serialises the tests of this file. `claim_due_sleeping_runs` claims rows
/// globally, so two tests running at once steal each other's runs.
static SERIAL: Mutex<()> = Mutex::const_new(());

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    let config = PoolConfig {
        acquire_timeout: Duration::from_secs(60),
        connect_timeout: Duration::from_secs(60),
        ..PoolConfig::default()
    };
    PostgresStore::with_config(&url, config)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run(name: &str) -> NewRun {
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
        max_cost_usd: None,
    }
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7())
}

/// Drain every already-due sleeping run so a test only sees the ones it armed.
async fn drain(store: &PostgresStore) {
    while !store
        .claim_due_sleeping_runs(100)
        .await
        .expect("drain")
        .is_empty()
    {}
}

/// Create a run walked to `Running`.
async fn running_run(store: &PostgresStore) -> Uuid {
    let run = store
        .create_run(new_run("signal-tests"))
        .await
        .expect("create run")
        .into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("to running");
    run.id
}

/// Create a `Sleeping` run whose wake-up time is `scheduled_at`.
async fn sleeping_run(store: &PostgresStore, scheduled_at: DateTime<Utc>) -> Uuid {
    let run_id = running_run(store).await;
    store
        .update_run(
            run_id,
            RunUpdate {
                status: Some(RunStatus::Sleeping),
                scheduled_at: Some(scheduled_at),
                ..RunUpdate::default()
            },
        )
        .await
        .expect("to sleeping");
    run_id
}

/// Open a `Running` signal step waiting for `(name, key)` on `run_id`.
async fn waiting_step(store: &PostgresStore, run_id: Uuid, name: &str, key: &str) -> Step {
    let step = store
        .create_step(NewStep {
            run_id,
            trace_id: step_trace_id(run_id, "wait", 0),
            name: "wait".to_string(),
            kind: StepKind::Signal,
            position: 0,
            input: Some(json!({"name": name, "key": key, "schema": {}})),
            is_error_handler: false,
        })
        .await
        .expect("create step");
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                started_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("to running");
    store
        .get_step(step.id)
        .await
        .expect("get step")
        .expect("step exists")
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_signal_duplicate_idempotency_id_returns_existing() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let idempotency_id = unique("delivery");
    let new_signal = |key: &str| NewSignal {
        name: "demo.done".to_string(),
        key: key.to_string(),
        payload: json!({"n": 1}),
        idempotency_id: Some(idempotency_id.clone()),
    };

    let first = store.insert_signal(new_signal("k1")).await.expect("insert");
    assert!(!first.is_duplicate());

    let second = store.insert_signal(new_signal("k2")).await.expect("insert");
    assert!(second.is_duplicate());
    assert_eq!(second.signal().id, first.signal().id);
    assert_eq!(second.signal().key, "k1");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_signal_list_for_key_and_waiters() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let name = unique("demo");
    store
        .insert_signal(NewSignal {
            name: name.clone(),
            key: "k1".to_string(),
            payload: json!({}),
            idempotency_id: None,
        })
        .await
        .expect("insert");

    let found = store
        .list_signals_for_key(&name, "k1", Utc::now() - TimeDelta::minutes(1))
        .await
        .expect("list");
    assert_eq!(found.len(), 1);

    let run_id = running_run(&store).await;
    let step = waiting_step(&store, run_id, &name, "k1").await;
    let waiters = store
        .list_signal_waiters(&name, "k1")
        .await
        .expect("waiters");
    assert_eq!(
        waiters.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![step.id]
    );

    store
        .update_run_status(run_id, RunStatus::Cancelled)
        .await
        .expect("cancel");
    let waiters = store
        .list_signal_waiters(&name, "k1")
        .await
        .expect("waiters");
    assert!(waiters.is_empty(), "a cancelled run no longer waits");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_signal_claim_due_sleeping_runs_is_exclusive() {
    let _serial = SERIAL.lock().await;
    let store = Arc::new(get_store().await);
    drain(&store).await;

    let mut expected = HashSet::new();
    for _ in 0..10 {
        expected.insert(sleeping_run(&store, Utc::now() - TimeDelta::seconds(30)).await);
    }
    let future = sleeping_run(&store, Utc::now() + TimeDelta::hours(1)).await;

    let mut tasks = JoinSet::new();
    for _ in 0..5 {
        let store = store.clone();
        tasks.spawn(async move {
            store
                .claim_due_sleeping_runs(10)
                .await
                .expect("claim")
                .into_iter()
                .map(|r| {
                    assert_eq!(r.status.state, RunStatus::Pending);
                    assert!(r.scheduled_at.is_none());
                    r.id
                })
                .collect::<Vec<Uuid>>()
        });
    }

    let mut claimed: Vec<Uuid> = Vec::new();
    while let Some(result) = tasks.join_next().await {
        claimed.extend(result.expect("task panicked"));
    }
    let unique_ids: HashSet<Uuid> = claimed.iter().copied().collect();
    assert_eq!(unique_ids.len(), claimed.len(), "a run was woken twice");
    assert_eq!(unique_ids, expected);

    let future = store.get_run(future).await.expect("get").expect("exists");
    assert_eq!(future.status.state, RunStatus::Sleeping);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_signal_resolution_records_signal_received_transition() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let name = unique("demo");

    let run_id = running_run(&store).await;
    let step = waiting_step(&store, run_id, &name, "k1").await;
    let waiting = store
        .suspend_run_on_signal(run_id, step.id, Utc::now() + TimeDelta::hours(1))
        .await
        .expect("suspend");
    assert!(waiting);
    let run = store.get_run(run_id).await.expect("get").expect("exists");
    assert_eq!(run.status.state, RunStatus::Sleeping);
    assert!(run.scheduled_at.is_some());

    let resolution = store
        .resolve_signal_step(step.id, json!({"timed_out": false}))
        .await
        .expect("resolve");
    assert_eq!(
        resolution,
        SignalStepResolution::Resolved {
            run_id,
            run_resumed: true,
        }
    );

    let run = store.get_run(run_id).await.expect("get").expect("exists");
    assert_eq!(run.status.state, RunStatus::Pending);
    assert!(run.scheduled_at.is_none());
    let resolved = store.get_step(step.id).await.expect("get").expect("exists");
    assert_eq!(resolved.status.state, StepStatus::Completed);
    assert_eq!(resolved.output, Some(json!({"timed_out": false})));

    let pool = PgPool::connect(&var("DATABASE_URL").expect("DATABASE_URL must be set"))
        .await
        .expect("connect");
    let events: i64 = query_scalar(
        r#"
        SELECT count(*) FROM lib_fsm.state_machine_event
        WHERE state_machine__id = $1 AND event = 'signal_received'
        "#,
    )
    .bind(run.status.state_machine_id)
    .fetch_one(&pool)
    .await
    .expect("count events");
    assert_eq!(events, 1);
    pool.close().await;

    let again = store
        .resolve_signal_step(step.id, json!({"timed_out": true}))
        .await
        .expect("resolve again");
    assert_eq!(
        again,
        SignalStepResolution::NotWaiting {
            output: Some(json!({"timed_out": false})),
        }
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_signal_suspend_after_resolution_schedules_now() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let name = unique("demo");

    let run_id = running_run(&store).await;
    let step = waiting_step(&store, run_id, &name, "k1").await;
    let resolution = store
        .resolve_signal_step(step.id, json!({"timed_out": false}))
        .await
        .expect("resolve");
    assert_eq!(
        resolution,
        SignalStepResolution::Resolved {
            run_id,
            run_resumed: false,
        }
    );

    let deadline = Utc::now() + TimeDelta::hours(1);
    let waiting = store
        .suspend_run_on_signal(run_id, step.id, deadline)
        .await
        .expect("suspend");
    assert!(!waiting);

    let run = store.get_run(run_id).await.expect("get").expect("exists");
    assert_eq!(run.status.state, RunStatus::Sleeping);
    assert!(run.scheduled_at.is_some_and(|at| at < deadline));

    let woken = store.claim_due_sleeping_runs(100).await.expect("claim");
    assert!(woken.iter().any(|r| r.id == run_id));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_signal_purge_removes_old_signals() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let name = unique("demo");
    store
        .insert_signal(NewSignal {
            name: name.clone(),
            key: "k1".to_string(),
            payload: json!({}),
            idempotency_id: None,
        })
        .await
        .expect("insert");

    let purged = store
        .purge_signals(Utc::now() + TimeDelta::seconds(1))
        .await
        .expect("purge");
    assert!(purged >= 1);

    let found = store
        .list_signals_for_key(&name, "k1", Utc::now() - TimeDelta::hours(1))
        .await
        .expect("list");
    assert!(found.is_empty());
}
