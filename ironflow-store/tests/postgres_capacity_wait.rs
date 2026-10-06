#![cfg(all(feature = "store-postgres", feature = "secret-store"))]

//! Integration tests for runs sleeping on provider capacity, on PostgreSQL.
//!
//! They cover the SQL paths the in-memory store cannot: the
//! `capacity_wait_kind` column set and cleared with the run status, and the
//! wake-up an account change applies to the runs sleeping on its kind.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres,secret-store --test postgres_capacity_wait -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use chrono::{DateTime, SubsecRound, TimeDelta, Utc};
use ironflow_store::crypto::KeyRing;
use ironflow_store::entities::{
    NewProviderAccount, NewRun, ProviderAccountUpdate, ProviderKind, RunStatus, RunUpdate,
    TriggerKind, provider_account_secret_key,
};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::provider_account_store::ProviderAccountStore;
use ironflow_store::store::RunStore;
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Serialises the tests of this file. `claim_due_sleeping_runs` claims rows
/// globally, so a test claiming runs would steal the ones another test armed.
static SERIAL: Mutex<()> = Mutex::const_new(());

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    let mut store = PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL");
    let spec = format!("1:{}", "aa".repeat(32));
    store.set_key_ring(KeyRing::from_spec(&spec, Some(1)).expect("valid ring"));
    store
}

/// A name unique across test runs sharing the database.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7().simple())
}

fn new_account(kind: &str, enabled: bool) -> NewProviderAccount {
    let id = Uuid::now_v7();
    let name = unique("capacity");
    NewProviderAccount {
        id,
        name: name.clone(),
        display_name: name,
        kind: kind.to_string(),
        secret_key: provider_account_secret_key(id),
        enabled,
        priority: 100,
        tags: Vec::new(),
        max_concurrency: None,
        alert_threshold: 0.8,
        expires_at: Utc::now() + TimeDelta::days(365),
        plan: None,
        created_by: None,
    }
}

/// One hour from now, in whole seconds so it survives the microsecond
/// precision of a Postgres timestamp unchanged.
fn in_one_hour() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(0) + TimeDelta::hours(1)
}

/// Create a `Sleeping` run waking at `scheduled_at`, waiting for `kind`
/// capacity when one is given.
async fn sleeping_run(
    store: &PostgresStore,
    scheduled_at: DateTime<Utc>,
    kind: Option<&str>,
) -> Uuid {
    let run = store
        .create_run(NewRun {
            workflow_name: "capacity-tests".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("create run")
        .into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("to running");
    store
        .update_run(
            run.id,
            RunUpdate {
                status: Some(RunStatus::Sleeping),
                scheduled_at: Some(scheduled_at),
                capacity_wait_kind: kind.map(ProviderKind::from),
                ..RunUpdate::default()
            },
        )
        .await
        .expect("to sleeping");
    run.id
}

/// The wake-up time of `run_id`.
async fn scheduled_at(store: &PostgresStore, run_id: Uuid) -> Option<DateTime<Utc>> {
    store
        .get_run(run_id)
        .await
        .expect("get run")
        .expect("run exists")
        .scheduled_at
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_capacity_wait_kind_is_persisted_while_sleeping() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let kind = unique("kind");
    let wake_at = in_one_hour();

    let run_id = sleeping_run(&store, wake_at, Some(&kind)).await;
    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Sleeping);
    assert_eq!(
        run.capacity_wait_kind,
        Some(ProviderKind::new(kind.as_str()))
    );

    store
        .update_run_status(run_id, RunStatus::Cancelled)
        .await
        .expect("to cancelled");
    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.capacity_wait_kind, None);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_claiming_a_capacity_sleeper_clears_its_kind() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let kind = unique("kind");

    let run_id = sleeping_run(&store, Utc::now() - TimeDelta::seconds(1), Some(&kind)).await;
    let mut woken = Vec::new();
    loop {
        let batch = store.claim_due_sleeping_runs(100).await.expect("claim");
        if batch.is_empty() {
            break;
        }
        woken.extend(batch.into_iter().map(|r| r.id));
    }
    assert!(woken.contains(&run_id));

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.capacity_wait_kind, None);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_creating_an_account_wakes_the_sleepers_of_its_kind() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let kind = unique("kind");
    let other_kind = unique("other");
    let wake_at = in_one_hour();

    let waiting = sleeping_run(&store, wake_at, Some(&kind)).await;
    let other = sleeping_run(&store, wake_at, Some(&other_kind)).await;
    let delayed = sleeping_run(&store, wake_at, None).await;

    store
        .create_provider_account(new_account(&kind, false))
        .await
        .expect("create disabled account");
    assert_eq!(scheduled_at(&store, waiting).await, Some(wake_at));

    store
        .create_provider_account(new_account(&kind, true))
        .await
        .expect("create account");
    let woken_at = scheduled_at(&store, waiting).await.expect("scheduled");
    assert!(woken_at < wake_at - TimeDelta::minutes(30), "{woken_at}");
    assert_eq!(scheduled_at(&store, other).await, Some(wake_at));
    assert_eq!(scheduled_at(&store, delayed).await, Some(wake_at));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_re_enabling_an_account_wakes_the_sleepers_of_its_kind() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    let kind = unique("kind");
    let wake_at = in_one_hour();

    let account = store
        .create_provider_account(new_account(&kind, false))
        .await
        .expect("create disabled account");
    let waiting = sleeping_run(&store, wake_at, Some(&kind)).await;

    store
        .update_provider_account(
            account.id,
            ProviderAccountUpdate {
                priority: Some(1),
                ..ProviderAccountUpdate::default()
            },
        )
        .await
        .expect("update priority");
    assert_eq!(scheduled_at(&store, waiting).await, Some(wake_at));

    store
        .update_provider_account(
            account.id,
            ProviderAccountUpdate {
                enabled: Some(true),
                ..ProviderAccountUpdate::default()
            },
        )
        .await
        .expect("enable");
    let woken_at = scheduled_at(&store, waiting).await.expect("scheduled");
    assert!(woken_at < wake_at - TimeDelta::minutes(30), "{woken_at}");
}
