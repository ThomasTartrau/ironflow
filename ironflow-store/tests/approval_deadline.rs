//! Integration tests for approval SLA deadlines on the in-memory store.
//!
//! The deadline lives on the approval step itself, so these tests drive
//! `create_step` / `update_step` / `claim_due_approval_deadlines` through the
//! public `RunStore` API and never reach into the store's internals.

use std::collections::HashMap;

use chrono::{TimeDelta, Utc};
use ironflow_store::prelude::*;
use serde_json::json;
use uuid::Uuid;

fn new_run(name: &str) -> NewRun {
    NewRun {
        created_by: None,
        workflow_name: name.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 3,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        idempotency_key: None,
        max_cost_usd: None,
    }
}

fn new_approval_step(run_id: Uuid, name: &str, position: u32) -> NewStep {
    NewStep {
        run_id,
        trace_id: step_trace_id(run_id, name, position),
        name: name.to_string(),
        kind: StepKind::Approval,
        position,
        input: Some(json!({"message": "Approve?"})),
        is_error_handler: false,
    }
}

/// Create an approval step already suspended on `AwaitingApproval`.
async fn awaiting_step(store: &InMemoryStore, run_id: Uuid, name: &str, position: u32) -> Step {
    let step = store
        .create_step(new_approval_step(run_id, name, position))
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
                ..StepUpdate::default()
            },
        )
        .await
        .expect("to awaiting approval");

    store.get_step(step.id).await.expect("get").expect("exists")
}

/// A store holding one run with one gate awaiting approval.
async fn gate() -> (InMemoryStore, Uuid, Step) {
    let store = InMemoryStore::new();
    let run = store
        .create_run(new_run("deploy"))
        .await
        .expect("create run")
        .into_run();
    let step = awaiting_step(&store, run.id, "prod-gate", 0).await;
    (store, run.id, step)
}

#[tokio::test]
async fn deadline_defaults_to_none_on_a_new_step() {
    let (store, run_id, step) = gate().await;

    assert!(step.approval_deadline_at.is_none());
    assert_eq!(step.approval_stage, 0);
    assert!(step.approval_assignee.is_none());

    let listed = store.list_steps(run_id).await.expect("list");
    assert!(listed[0].approval_deadline_at.is_none());
}

#[tokio::test]
async fn deadline_is_persisted_and_read_back() {
    let (store, run_id, step) = gate().await;
    let at = Utc::now() + TimeDelta::seconds(3600);

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(at),
                approval_stage: Some(1),
                approval_assignee: Some(Assignee::group("sre-oncall")),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("arm timer");

    let fetched = store.get_step(step.id).await.expect("get").expect("exists");
    assert_eq!(fetched.approval_deadline_at, Some(at));
    assert_eq!(fetched.approval_stage, 1);
    assert_eq!(
        fetched.approval_assignee,
        Some(Assignee::group("sre-oncall"))
    );

    let listed = store.list_steps(run_id).await.expect("list");
    assert_eq!(listed[0].approval_deadline_at, Some(at));
}

#[tokio::test]
async fn deadline_claim_returns_only_expired_steps() {
    let store = InMemoryStore::new();
    let run = store
        .create_run(new_run("deploy"))
        .await
        .expect("create run")
        .into_run();

    let expired = awaiting_step(&store, run.id, "expired-gate", 0).await;
    let future = awaiting_step(&store, run.id, "future-gate", 1).await;

    store
        .update_step(
            expired.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() - TimeDelta::seconds(5)),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("arm expired");
    store
        .update_step(
            future.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() + TimeDelta::seconds(3600)),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("arm future");

    let claimed = store.claim_due_approval_deadlines(10).await.expect("claim");

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, expired.id);
    // The claim keeps the deadline that fired on the returned copy.
    assert!(claimed[0].approval_deadline_at.is_some());
}

#[tokio::test]
async fn deadline_claim_skips_steps_not_awaiting_approval() {
    let (store, run_id, step) = gate().await;

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() - TimeDelta::seconds(5)),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("arm timer");
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("complete");

    let claimed = store.claim_due_approval_deadlines(10).await.expect("claim");
    assert!(claimed.is_empty());

    // The gate is gone but the run is untouched.
    let listed = store.list_steps(run_id).await.expect("list");
    assert_eq!(listed[0].status.state, StepStatus::Completed);
}

#[tokio::test]
async fn deadline_claim_clears_the_timer_so_a_deadline_fires_once() {
    let (store, _run_id, step) = gate().await;

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() - TimeDelta::seconds(5)),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("arm timer");

    let first = store.claim_due_approval_deadlines(10).await.expect("claim");
    assert_eq!(first.len(), 1);

    let second = store.claim_due_approval_deadlines(10).await.expect("claim");
    assert!(second.is_empty());

    let stored = store.get_step(step.id).await.expect("get").expect("exists");
    assert!(stored.approval_deadline_at.is_none());
    assert_eq!(stored.status.state, StepStatus::AwaitingApproval);
}

#[tokio::test]
async fn deadline_claim_respects_the_limit_and_orders_by_expiry() {
    let store = InMemoryStore::new();
    let run = store
        .create_run(new_run("deploy"))
        .await
        .expect("create run")
        .into_run();

    let now = Utc::now();
    let mut ids = Vec::new();
    // Armed oldest-last so ordering cannot come from insertion order.
    for (position, age) in [(0u32, 5i64), (1, 30), (2, 60)] {
        let step = awaiting_step(&store, run.id, &format!("gate-{position}"), position).await;
        store
            .update_step(
                step.id,
                StepUpdate {
                    approval_deadline_at: Some(now - TimeDelta::seconds(age)),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("arm timer");
        ids.push((age, step.id));
    }

    let claimed = store.claim_due_approval_deadlines(2).await.expect("claim");

    assert_eq!(claimed.len(), 2);
    // Oldest deadline first: the 60 s-old gate, then the 30 s-old one.
    assert_eq!(claimed[0].id, ids[2].1);
    assert_eq!(claimed[1].id, ids[1].1);

    let rest = store.claim_due_approval_deadlines(10).await.expect("claim");
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].id, ids[0].1);
}

#[tokio::test]
async fn deadline_reset_reschedules_and_bumps_the_stage() {
    let (store, _run_id, step) = gate().await;

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() - TimeDelta::seconds(5)),
                approval_stage: Some(0),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("arm timer");

    store.claim_due_approval_deadlines(10).await.expect("claim");

    let next = Utc::now() + TimeDelta::seconds(600);
    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(next),
                approval_stage: Some(1),
                approval_assignee: Some(Assignee::group("sre-oncall")),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("reschedule");

    let stored = store.get_step(step.id).await.expect("get").expect("exists");
    assert_eq!(stored.approval_deadline_at, Some(next));
    assert_eq!(stored.approval_stage, 1);
    assert_eq!(
        stored.approval_assignee,
        Some(Assignee::group("sre-oncall"))
    );
    assert!(
        store
            .claim_due_approval_deadlines(10)
            .await
            .expect("claim")
            .is_empty()
    );
}

#[tokio::test]
async fn deadline_clear_flag_removes_the_timer() {
    let (store, _run_id, step) = gate().await;

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() + TimeDelta::seconds(600)),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("arm timer");

    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await
        .expect("clear timer");

    let stored = store.get_step(step.id).await.expect("get").expect("exists");
    assert!(stored.approval_deadline_at.is_none());
}

#[tokio::test]
async fn deadline_clear_flag_wins_over_a_new_deadline() {
    let (store, _run_id, step) = gate().await;

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() + TimeDelta::seconds(600)),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await
        .expect("update");

    let stored = store.get_step(step.id).await.expect("get").expect("exists");
    assert!(stored.approval_deadline_at.is_none());
}

#[tokio::test]
async fn deadline_update_leaves_other_step_fields_untouched() {
    let (store, _run_id, step) = gate().await;

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() + TimeDelta::seconds(600)),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("arm timer");

    let stored = store.get_step(step.id).await.expect("get").expect("exists");
    assert_eq!(stored.status.state, StepStatus::AwaitingApproval);
    assert_eq!(stored.name, step.name);
    assert_eq!(stored.kind, step.kind);
    assert_eq!(stored.position, step.position);
    assert_eq!(stored.input, step.input);
    assert_eq!(stored.output, step.output);
    assert_eq!(stored.error, step.error);
    assert_eq!(stored.started_at, step.started_at);
    assert_eq!(stored.completed_at, step.completed_at);
}
