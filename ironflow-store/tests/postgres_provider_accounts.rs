#![cfg(all(feature = "store-postgres", feature = "secret-store"))]

//! Integration tests for Provider Accounts on the PostgreSQL store.
//!
//! They cover the SQL paths the in-memory store cannot: the unique name
//! constraint, the `observed_at` guard of the window upsert, the running-step
//! count joined on `lib_fsm`, and the cascades of an account deletion.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres,secret-store --test postgres_provider_accounts -- --ignored
//! ```

use std::collections::HashMap;
use std::env;

use chrono::{DateTime, TimeDelta, Utc};
use ironflow_store::crypto::KeyRing;
use ironflow_store::entities::{
    AccountWindowStatus, NewAccountWindow, NewProviderAccount, NewProviderAccountObservation,
    NewRun, NewStep, ProviderAccountUpdate, StepKind, StepStatus, StepUpdate, TriggerKind,
    provider_account_secret_key, step_trace_id,
};
use ironflow_store::error::StoreError;
use ironflow_store::postgres::PostgresStore;
use ironflow_store::provider_account_store::ProviderAccountStore;
use ironflow_store::secret_store::SecretStore;
use ironflow_store::store::RunStore;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn database_url() -> String {
    env::var("DATABASE_URL").expect("DATABASE_URL must be set")
}

async fn get_store() -> PostgresStore {
    let mut store = PostgresStore::new(&database_url())
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

fn new_account(name: &str, kind: &str) -> NewProviderAccount {
    let id = Uuid::now_v7();
    NewProviderAccount {
        id,
        name: name.to_string(),
        display_name: name.to_string(),
        kind: kind.to_string(),
        secret_key: provider_account_secret_key(id),
        enabled: true,
        priority: 100,
        tags: vec!["perso".to_string()],
        max_concurrency: Some(2),
        alert_threshold: 0.8,
        expires_at: Utc::now() + TimeDelta::days(365),
        plan: Some("max".to_string()),
        created_by: None,
    }
}

fn window(name: &str, utilization: f64, observed_at: DateTime<Utc>) -> NewAccountWindow {
    NewAccountWindow {
        window: name.to_string(),
        utilization,
        resets_at: Some(observed_at + TimeDelta::hours(2)),
        status: AccountWindowStatus::Allowed,
        model_scope: None,
        observed_at,
    }
}

fn new_run(name: &str) -> NewRun {
    NewRun {
        created_by: None,
        workflow_name: name.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        idempotency_key: None,
        concurrency_key: None,
        max_cost_usd: None,
    }
}

#[tokio::test]
#[ignore]
async fn postgres_provider_account_crud_and_duplicate_name() {
    let store = get_store().await;
    let name = unique("crud");
    let account = store
        .create_provider_account(new_account(&name, "claude_subscription"))
        .await
        .unwrap();
    assert_eq!(account.tags, vec!["perso".to_string()]);
    assert_eq!(account.max_concurrency, Some(2));

    let err = store
        .create_provider_account(new_account(&name, "claude_subscription"))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::DuplicateProviderAccount(_)));

    let found = store.find_provider_account_by_name(&name).await.unwrap();
    assert_eq!(found.map(|a| a.id), Some(account.id));

    let updated = store
        .update_provider_account(
            account.id,
            ProviderAccountUpdate {
                max_concurrency: Some(None),
                tags: Some(vec!["team".to_string()]),
                ..ProviderAccountUpdate::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.max_concurrency, None);
    assert_eq!(updated.tags, vec!["team".to_string()]);

    let kind = unique("kind");
    store
        .create_provider_account(new_account(&unique("listed"), &kind))
        .await
        .unwrap();
    let page = store
        .list_provider_accounts(Some(kind), 1, 20)
        .await
        .unwrap();
    assert_eq!(page.total, 1);

    assert!(store.delete_provider_account(account.id).await.unwrap());
    assert!(
        store
            .get_provider_account(account.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
#[ignore]
async fn postgres_provider_account_observation_guard_and_history() {
    let store = get_store().await;
    let account = store
        .create_provider_account(new_account(&unique("obs"), "claude_subscription"))
        .await
        .unwrap();
    let newer = Utc::now();
    let older = newer - TimeDelta::minutes(30);
    for (utilization, at) in [(0.7, newer), (0.1, older)] {
        store
            .record_provider_account_observation(
                account.id,
                NewProviderAccountObservation {
                    windows: vec![window("five_hour", utilization, at)],
                    auth_failed: false,
                },
            )
            .await
            .unwrap();
    }
    let windows = store
        .list_provider_account_windows(vec![account.id])
        .await
        .unwrap();
    assert_eq!(windows.len(), 1);
    assert!((windows[0].utilization - 0.7).abs() < 1e-9);

    let history = store
        .list_provider_account_usage(account.id, newer - TimeDelta::hours(1))
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(history[0].observed_at <= history[1].observed_at);

    store
        .record_provider_account_observation(
            account.id,
            NewProviderAccountObservation {
                windows: Vec::new(),
                auth_failed: true,
            },
        )
        .await
        .unwrap();
    let failed = store
        .get_provider_account(account.id)
        .await
        .unwrap()
        .unwrap();
    assert!(failed.auth_failed_at.is_some());

    let purged = store
        .purge_provider_account_usage(newer - TimeDelta::minutes(1))
        .await
        .unwrap();
    assert!(purged >= 1);
    store.delete_provider_account(account.id).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn postgres_provider_account_windows_round_trip_scope_and_status() {
    let store = get_store().await;
    let account = store
        .create_provider_account(new_account(&unique("scope"), "claude_subscription"))
        .await
        .unwrap();
    let at = Utc::now();
    let scoped = NewAccountWindow {
        status: AccountWindowStatus::AllowedWarning,
        model_scope: Some("opus".to_string()),
        ..window("seven_day", 0.9, at)
    };
    let global = NewAccountWindow {
        status: AccountWindowStatus::Rejected,
        ..window("seven_day", 1.0, at)
    };
    let windows = store
        .record_provider_account_observation(
            account.id,
            NewProviderAccountObservation {
                windows: vec![scoped, global],
                auth_failed: false,
            },
        )
        .await
        .unwrap();

    // `model_scope = ''` is the stored form of `None` and sorts first.
    let read: Vec<_> = windows
        .iter()
        .map(|w| (w.window.as_str(), w.model_scope.as_deref(), w.status))
        .collect();
    assert_eq!(
        read,
        vec![
            ("seven_day", None, AccountWindowStatus::Rejected),
            (
                "seven_day",
                Some("opus"),
                AccountWindowStatus::AllowedWarning
            ),
        ]
    );

    let history = store
        .list_provider_account_usage(account.id, at - TimeDelta::minutes(1))
        .await
        .unwrap();
    let mut scopes: Vec<_> = history
        .iter()
        .map(|p| (p.model_scope.as_deref(), p.status))
        .collect();
    scopes.sort_by_key(|(scope, _)| *scope);
    assert_eq!(
        scopes,
        vec![
            (None, AccountWindowStatus::Rejected),
            (Some("opus"), AccountWindowStatus::AllowedWarning),
        ]
    );
    store.delete_provider_account(account.id).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn postgres_list_provider_accounts_total_past_the_last_page() {
    let store = get_store().await;
    let kind = unique("kind");
    for _ in 0..3 {
        store
            .create_provider_account(new_account(&unique("paged"), &kind))
            .await
            .unwrap();
    }
    let page = store
        .list_provider_accounts(Some(kind.clone()), 2, 2)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.total, 3);

    // The total no longer comes from the returned rows: an empty page still carries it.
    let past = store
        .list_provider_accounts(Some(kind), 5, 2)
        .await
        .unwrap();
    assert!(past.items.is_empty());
    assert_eq!(past.total, 3);
}

#[tokio::test]
#[ignore]
async fn postgres_provider_account_candidates_count_running_steps_and_delete_nulls_steps() {
    let store = get_store().await;
    let kind = unique("kind");
    let active = store
        .create_provider_account(new_account(&unique("active"), &kind))
        .await
        .unwrap();
    store
        .create_provider_account(NewProviderAccount {
            enabled: false,
            ..new_account(&unique("disabled"), &kind)
        })
        .await
        .unwrap();

    let run = store
        .create_run(new_run(&unique("wf")))
        .await
        .unwrap()
        .into_run();
    // Give the run a live lease without picking it: picking could steal a
    // pending run of another test sharing the database.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to PostgreSQL");
    sqlx::query(
        "UPDATE ironflow.runs SET lease_expires_at = NOW() + INTERVAL '90 seconds' WHERE id = $1",
    )
    .bind(run.id)
    .execute(&pool)
    .await
    .unwrap();
    let step = store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "agent", 0),
            name: "agent".to_string(),
            kind: StepKind::Agent,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .unwrap();
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                account_id: Some(active.id),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();

    let candidates = store
        .list_provider_account_candidates(kind.clone())
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].account.id, active.id);
    assert_eq!(candidates[0].running_steps, 1);

    assert!(store.delete_provider_account(active.id).await.unwrap());
    let step = store.get_step(step.id).await.unwrap().unwrap();
    assert_eq!(step.account_id, None);
}

#[tokio::test]
#[ignore]
async fn postgres_list_secrets_hides_provider_account_credentials() {
    let store = get_store().await;
    let key = provider_account_secret_key(Uuid::now_v7());
    store.set_secret(&key, "sk-ant-oat01-hidden").await.unwrap();
    let page = store.list_secrets("accounts/", 1, 100).await.unwrap();
    assert_eq!(page.total, 0);
    assert!(store.get_secret(&key).await.unwrap().is_some());
    store.delete_secret(&key).await.unwrap();
}
