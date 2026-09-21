#![cfg(feature = "store-postgres")]

//! Integration tests for approval SLA deadlines on PostgreSQL.
//!
//! These tests need a real database (`DATABASE_URL`) because the guarantee they
//! check — a deadline fires exactly once across several escalators — lives in
//! `FOR UPDATE SKIP LOCKED`, which the in-memory store cannot reproduce.
//!
//! Run them with:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store --features store-postgres --test postgres_approval_deadline -- --ignored
//! ```

use std::collections::{HashMap, HashSet};
use std::env::var;
use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use ironflow_store::entities::{
    NewRun, NewStep, RunStatus, StepKind, StepStatus, StepUpdate, TriggerKind, step_trace_id,
};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::store::RunStore;
use serde_json::json;
use tokio::task::JoinSet;
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
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

/// Drain every already-expired gate so a test only sees the ones it armed.
async fn drain(store: &PostgresStore) {
    while !store
        .claim_due_approval_deadlines(100)
        .await
        .expect("drain")
        .is_empty()
    {}
}

/// Arm `count` gates whose deadline has already passed, returning their IDs.
async fn arm_expired_gates(store: &PostgresStore, count: usize) -> HashSet<Uuid> {
    let run = store
        .create_run(new_run("deadline-concurrency"))
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

    let mut ids = HashSet::new();
    for position in 0..count {
        let name = format!("gate-{position}");
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, &name, position as u32),
                name,
                kind: StepKind::Approval,
                position: position as u32,
                input: Some(json!({"message": "Approve?"})),
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
            .expect("to running");
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    approval_deadline_at: Some(Utc::now() - TimeDelta::seconds(60)),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("arm timer");

        ids.insert(step.id);
    }
    ids
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn deadline_claim_is_exclusive_across_concurrent_claimers() {
    let store = Arc::new(get_store().await);
    drain(&store).await;

    let expected = arm_expired_gates(&store, 12).await;

    let mut tasks = JoinSet::new();
    for _ in 0..6 {
        let store = store.clone();
        tasks.spawn(async move {
            store
                .claim_due_approval_deadlines(12)
                .await
                .expect("claim")
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<Uuid>>()
        });
    }

    let mut claimed: Vec<Uuid> = Vec::new();
    while let Some(result) = tasks.join_next().await {
        claimed.extend(result.expect("task panicked"));
    }

    let unique: HashSet<Uuid> = claimed.iter().copied().collect();
    assert_eq!(
        unique.len(),
        claimed.len(),
        "a deadline was claimed by two escalators at once"
    );
    for id in &expected {
        assert!(unique.contains(id), "gate {id} was never claimed");
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn deadline_claim_clears_the_timer_in_postgres() {
    let store = get_store().await;
    drain(&store).await;

    let expected = arm_expired_gates(&store, 1).await;
    let id = *expected.iter().next().expect("one gate");

    let first = store.claim_due_approval_deadlines(10).await.expect("claim");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].id, id);
    assert!(first[0].approval_deadline_at.is_some());

    let second = store.claim_due_approval_deadlines(10).await.expect("claim");
    assert!(second.is_empty());

    let stored = store.get_step(id).await.expect("get").expect("exists");
    assert!(stored.approval_deadline_at.is_none());
    assert_eq!(stored.status.state, StepStatus::AwaitingApproval);
}
