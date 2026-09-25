#![cfg(feature = "store-postgres")]

//! Integration tests for dynamic approval rules on PostgreSQL.
//!
//! These tests need a real database (`DATABASE_URL`) because what they check --
//! the JSONB columns on `ironflow.steps`, the `@>` guard that keeps a vote
//! idempotent, and the cascade from `iam.users` onto `iam.user_groups` -- lives
//! in the schema and in SQL, not in Rust.
//!
//! Run them with:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store --features store-postgres --test postgres_approval_rules -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use chrono::Utc;
use ironflow_store::entities::{
    ApprovalRequirement, ApprovalRuleEvaluation, NewRun, NewStep, NewUser, Step, StepApproval,
    StepKind, StepUpdate, TriggerKind, step_trace_id,
};
use ironflow_store::error::StoreError;
use ironflow_store::postgres::PostgresStore;
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;
use serde_json::json;
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

/// Create one run holding one approval step.
async fn gate(store: &PostgresStore) -> Step {
    let run = store
        .create_run(NewRun {
            workflow_name: "approval-rules".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({"amount": 15000}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("create run")
        .into_run();

    store
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
        .expect("create step")
}

/// Create a real user. The suffix keeps reruns from colliding.
async fn create_user(store: &PostgresStore, prefix: &str) -> Uuid {
    let suffix = Uuid::now_v7();
    store
        .create_user(NewUser {
            email: format!("{prefix}-{suffix}@example.com"),
            username: format!("{prefix}-{suffix}"),
            password_hash: "not-a-real-hash".to_string(),
            is_admin: Some(false),
        })
        .await
        .expect("create user")
        .id
}

fn vote(user_id: Uuid, name: &str) -> StepApproval {
    StepApproval {
        user_id,
        approved_by: name.to_string(),
        at: Utc::now(),
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn requirement_roundtrips_through_update_step() {
    let store = get_store().await;
    let step = gate(&store).await;
    assert!(step.approval_requirement.is_none());
    assert!(step.approvals.is_empty());

    let requirement = ApprovalRequirement {
        rule_index: Some(0),
        condition: Some("payload.amount > 10000".to_string()),
        required_approvers: 2,
        approver_groups: vec!["finance".to_string()],
        evaluated: vec![ApprovalRuleEvaluation {
            index: 0,
            condition: "payload.amount > 10000".to_string(),
            matched: true,
        }],
    };

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_requirement: Some(requirement.clone()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("set requirement");

    let fetched = store.get_step(step.id).await.expect("get").expect("exists");
    assert_eq!(fetched.approval_requirement, Some(requirement.clone()));

    let listed = store.list_steps(step.run_id).await.expect("list");
    assert_eq!(listed[0].approval_requirement, Some(requirement));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn record_step_approval_is_idempotent_per_user() {
    let store = get_store().await;
    let step = gate(&store).await;
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
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn record_step_approval_on_unknown_step_is_not_found() {
    let store = get_store().await;

    let err = store
        .record_step_approval(Uuid::now_v7(), vote(Uuid::now_v7(), "alice"))
        .await
        .expect_err("unknown step");

    assert!(matches!(err, StoreError::StepNotFound(_)));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn user_groups_persist_and_cascade_on_user_delete() {
    let store = get_store().await;
    let user = create_user(&store, "groups").await;

    let groups = store
        .set_user_groups(
            user,
            vec!["sre".to_string(), "finance".to_string(), "sre".to_string()],
        )
        .await
        .expect("set groups");
    assert_eq!(groups, vec!["finance".to_string(), "sre".to_string()]);
    assert_eq!(
        store.list_user_groups(user).await.expect("list"),
        vec!["finance".to_string(), "sre".to_string()]
    );

    let replaced = store
        .set_user_groups(user, vec!["legal".to_string()])
        .await
        .expect("replace groups");
    assert_eq!(replaced, vec!["legal".to_string()]);

    store.delete_user(user).await.expect("delete user");
    assert!(store.list_user_groups(user).await.expect("list").is_empty());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn set_user_groups_on_unknown_user_is_not_found() {
    let store = get_store().await;

    let err = store
        .set_user_groups(Uuid::now_v7(), vec!["finance".to_string()])
        .await
        .expect_err("unknown user");

    assert!(matches!(err, StoreError::UserNotFound(_)));
}
