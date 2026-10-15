#![cfg(feature = "store-postgres")]

//! Integration tests binding an approval gate's assignee to a user id on PostgreSQL.
//!
//! The binding is resolved inside the `UPDATE` that opens or reassigns the
//! gate, by a subquery on `iam.users`, so only a real database exercises it.
//!
//! Run them with:
//!
//! ```sh
//! scripts/test-postgres.sh -p ironflow-store --features store-postgres \
//!     --test postgres_approval_assignee -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;
use std::time::Duration;

use ironflow_store::entities::{
    Assignee, NewRun, NewStep, NewUser, RunStatus, StepKind, StepStatus, StepUpdate, TriggerKind,
    User, step_trace_id,
};
use ironflow_store::postgres::{PoolConfig, PostgresStore};
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;
use serde_json::json;
use uuid::Uuid;

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

/// A username no other test, nor an earlier run on the same database, uses.
fn unique_username(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7().simple())
}

async fn create_member(store: &PostgresStore, username: &str) -> User {
    store
        .create_user(NewUser {
            email: format!("{username}@example.com"),
            username: username.to_string(),
            password_hash: "hash".to_string(),
            is_admin: Some(false),
        })
        .await
        .expect("create user")
}

fn new_run() -> NewRun {
    NewRun {
        workflow_name: "assignee-binding".to_string(),
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
    }
}

/// Open an approval gate assigned to `assignee` and return the step id.
async fn open_gate(store: &PostgresStore, assignee: Assignee) -> Uuid {
    let run = store
        .create_run(new_run())
        .await
        .expect("create run")
        .into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("to running");
    store
        .update_run_status(run.id, RunStatus::AwaitingApproval)
        .await
        .expect("to awaiting approval");

    let step = store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "gate", 0),
            name: "gate".to_string(),
            kind: StepKind::Approval,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("create step");
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("step to running");
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::AwaitingApproval),
                approval_assignee: Some(assignee),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("open gate");
    step.id
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn gate_assigned_to_an_existing_user_is_bound_to_their_id() {
    let store = get_store().await;
    let username = unique_username("alice");
    let alice = create_member(&store, &username).await;

    let step_id = open_gate(&store, Assignee::user(&username)).await;

    let step = store
        .get_step(step_id)
        .await
        .expect("get step")
        .expect("step exists");
    assert_eq!(step.approval_assignee, Some(Assignee::user(&username)));
    assert_eq!(step.approval_assignee_user_id, Some(alice.id));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn account_created_after_the_gate_opened_is_not_bound_to_it() {
    let store = get_store().await;
    let username = unique_username("alice");

    let step_id = open_gate(&store, Assignee::user(&username)).await;
    create_member(&store, &username).await;

    let step = store
        .get_step(step_id)
        .await
        .expect("get step")
        .expect("step exists");
    assert_eq!(step.approval_assignee, Some(Assignee::user(&username)));
    assert_eq!(step.approval_assignee_user_id, None);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn reassigning_the_gate_to_a_group_clears_the_bound_user() {
    let store = get_store().await;
    let username = unique_username("alice");
    let alice = create_member(&store, &username).await;
    let step_id = open_gate(&store, Assignee::user(&username)).await;
    let bound = store
        .get_step(step_id)
        .await
        .expect("get step")
        .expect("step exists");
    assert_eq!(bound.approval_assignee_user_id, Some(alice.id));

    store
        .update_step(
            step_id,
            StepUpdate {
                approval_assignee: Some(Assignee::group("sre-oncall")),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("reassign to a group");

    let step = store
        .get_step(step_id)
        .await
        .expect("get step")
        .expect("step exists");
    assert_eq!(step.approval_assignee, Some(Assignee::group("sre-oncall")));
    assert_eq!(step.approval_assignee_user_id, None);
}
