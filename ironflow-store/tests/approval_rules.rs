//! Integration tests for multi-approver gates on the in-memory store.
//!
//! Covers the persisted approval requirement, the vote log of a gate and the
//! user group membership used to restrict who may vote.

use std::collections::HashMap;

use chrono::Utc;
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

/// A store holding one run with one approval step.
async fn gate() -> (InMemoryStore, Uuid, Step) {
    let store = InMemoryStore::new();
    let run = store
        .create_run(new_run("payments"))
        .await
        .expect("create run")
        .into_run();
    let step = store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "finance-gate", 0),
            name: "finance-gate".to_string(),
            kind: StepKind::Approval,
            position: 0,
            input: Some(json!({"message": "Approve?"})),
            is_error_handler: false,
        })
        .await
        .expect("create step");
    (store, run.id, step)
}

fn requirement() -> ApprovalRequirement {
    ApprovalRequirement {
        reason: Some("amount > 10k".to_string()),
        required_approvers: 2,
        approver_groups: vec!["finance".to_string()],
    }
}

fn vote(user_id: Uuid, name: &str) -> StepApproval {
    StepApproval {
        user_id,
        approved_by: name.to_string(),
        at: Utc::now(),
    }
}

fn new_user(name: &str) -> NewUser {
    NewUser {
        email: format!("{name}@example.com"),
        username: name.to_string(),
        password_hash: "hash".to_string(),
        is_admin: None,
    }
}

#[tokio::test]
async fn a_new_step_has_no_requirement_and_no_votes() {
    let (_store, _run_id, step) = gate().await;

    assert!(step.approval_requirement.is_none());
    assert!(step.approvals.is_empty());
}

#[tokio::test]
async fn requirement_roundtrips_through_update_step() {
    let (store, run_id, step) = gate().await;

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_requirement: Some(requirement()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("set requirement");

    let fetched = store.get_step(step.id).await.expect("get").expect("exists");
    assert_eq!(fetched.approval_requirement, Some(requirement()));

    let listed = store.list_steps(run_id).await.expect("list");
    assert_eq!(listed[0].approval_requirement, Some(requirement()));
}

#[tokio::test]
async fn an_update_without_requirement_keeps_the_stored_one() {
    let (store, _run_id, step) = gate().await;
    store
        .update_step(
            step.id,
            StepUpdate {
                approval_requirement: Some(requirement()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("set requirement");

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_stage: Some(1),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("unrelated update");

    let fetched = store.get_step(step.id).await.expect("get").expect("exists");
    assert_eq!(fetched.approval_requirement, Some(requirement()));
}

#[tokio::test]
async fn record_step_approval_is_idempotent_per_user() {
    let (store, _run_id, step) = gate().await;
    let alice = Uuid::now_v7();
    let bob = Uuid::now_v7();

    let first = store
        .record_step_approval(step.id, vote(alice, "alice"))
        .await
        .expect("first vote");
    assert_eq!(first.approvals.len(), 1);

    let duplicate = store
        .record_step_approval(step.id, vote(alice, "alice"))
        .await
        .expect("duplicate vote");
    assert_eq!(duplicate.approvals.len(), 1);

    let second = store
        .record_step_approval(step.id, vote(bob, "bob"))
        .await
        .expect("second vote");
    let voters: Vec<Uuid> = second.approvals.iter().map(|a| a.user_id).collect();
    assert_eq!(voters, vec![alice, bob]);

    let fetched = store.get_step(step.id).await.expect("get").expect("exists");
    assert_eq!(fetched.approvals, second.approvals);
}

#[tokio::test]
async fn record_step_approval_on_unknown_step_is_not_found() {
    let store = InMemoryStore::new();

    let err = store
        .record_step_approval(Uuid::now_v7(), vote(Uuid::now_v7(), "alice"))
        .await
        .expect_err("unknown step");

    assert!(matches!(err, StoreError::StepNotFound(_)));
}

#[tokio::test]
async fn user_groups_are_persisted_sorted_and_deduplicated() {
    let store = InMemoryStore::new();
    let user = store.create_user(new_user("alice")).await.expect("user");

    let groups = store
        .set_user_groups(
            user.id,
            vec![
                "sre".to_string(),
                "finance".to_string(),
                "finance".to_string(),
            ],
        )
        .await
        .expect("set groups");

    assert_eq!(groups, vec!["finance".to_string(), "sre".to_string()]);
    assert_eq!(
        store.list_user_groups(user.id).await.expect("list"),
        vec!["finance".to_string(), "sre".to_string()]
    );
}

#[tokio::test]
async fn user_groups_are_removed_with_the_user() {
    let store = InMemoryStore::new();
    let user = store.create_user(new_user("alice")).await.expect("user");
    store
        .set_user_groups(user.id, vec!["finance".to_string()])
        .await
        .expect("set groups");

    store.delete_user(user.id).await.expect("delete");

    assert!(
        store
            .list_user_groups(user.id)
            .await
            .expect("list")
            .is_empty()
    );
}

#[tokio::test]
async fn set_user_groups_on_unknown_user_is_not_found() {
    let store = InMemoryStore::new();

    let err = store
        .set_user_groups(Uuid::now_v7(), vec!["finance".to_string()])
        .await
        .expect_err("unknown user");

    assert!(matches!(err, StoreError::UserNotFound(_)));
}
