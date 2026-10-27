#![cfg(feature = "store-postgres")]

//! Integration tests for the agent role of a step on the PostgreSQL store.
//!
//! The role is read from the `role` key of the persisted agent step input,
//! indexed by `idx_steps_agent_role`. These tests also prove that migration
//! applies. Each test uses role names of its own: the database is shared with
//! the other suites running in parallel.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_agent_role -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use chrono::{Duration, Utc};
use ironflow_store::entities::{
    AgentRoleStats, NewRun, NewStep, Step, StepKind, StepStatus, StepUpdate, TriggerKind,
    step_trace_id,
};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::store::RunStore;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run(workflow: &str) -> NewRun {
    NewRun {
        workflow_name: workflow.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
        worker_tags: Vec::new(),
        priority: 0,
    }
}

/// A role name no other test uses.
fn unique_role(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7())
}

/// Input of an agent step, as the engine persists it.
fn agent_input(role: Option<&str>) -> Value {
    match role {
        Some(role) => json!({"type": "agent", "prompt": "p", "role": role}),
        None => json!({"type": "agent", "prompt": "p"}),
    }
}

async fn create_run(store: &PostgresStore, workflow: &str) -> Uuid {
    store
        .create_run(new_run(workflow))
        .await
        .expect("create run")
        .into_run()
        .id
}

async fn create_step(
    store: &PostgresStore,
    run_id: Uuid,
    name: &str,
    kind: StepKind,
    input: Value,
) -> Step {
    store
        .create_step(NewStep {
            run_id,
            trace_id: step_trace_id(run_id, name, 0),
            name: name.to_string(),
            kind,
            position: 0,
            input: Some(input),
            is_error_handler: false,
        })
        .await
        .expect("create step")
}

/// Runs a step to `status` with a cost and a duration.
async fn finish_step(
    store: &PostgresStore,
    step_id: Uuid,
    status: StepStatus,
    cost_usd: Decimal,
    duration_ms: u64,
) {
    store
        .update_step(
            step_id,
            StepUpdate {
                status: Some(StepStatus::Running),
                started_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("start step");
    store
        .update_step(
            step_id,
            StepUpdate {
                status: Some(status),
                cost_usd: Some(cost_usd),
                duration_ms: Some(duration_ms),
                completed_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("finish step");
}

/// The stats of `role`, among every role of the shared database.
fn stats_of<'a>(stats: &'a [AgentRoleStats], role: &str) -> Option<&'a AgentRoleStats> {
    stats.iter().find(|s| s.agent_role == role)
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_role_step_reads_back_its_role() {
    let store = get_store().await;
    let role = unique_role("reader");
    let run_id = create_run(&store, "agent-role").await;
    let created = create_step(
        &store,
        run_id,
        "review",
        StepKind::Agent,
        agent_input(Some(&role)),
    )
    .await;
    assert_eq!(created.agent_role.as_deref(), Some(role.as_str()));

    let fetched = store
        .get_step(created.id)
        .await
        .expect("get step")
        .expect("step exists");
    assert_eq!(fetched.agent_role.as_deref(), Some(role.as_str()));

    let listed = store.list_steps(run_id).await.expect("list steps");
    assert_eq!(listed[0].agent_role.as_deref(), Some(role.as_str()));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn an_agent_step_without_role_has_none() {
    let store = get_store().await;
    let run_id = create_run(&store, "agent-role").await;
    let step = create_step(&store, run_id, "plain", StepKind::Agent, agent_input(None)).await;
    assert_eq!(step.agent_role, None);

    let shell = create_step(
        &store,
        run_id,
        "ls",
        StepKind::Shell,
        json!({"type": "shell", "command": "ls", "role": "Reviewer"}),
    )
    .await;
    assert_eq!(shell.agent_role, None, "only agent steps have a role");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn steps_by_role_span_runs_and_workflows() {
    let store = get_store().await;
    let role = unique_role("spanner");
    let other = unique_role("other");
    let deploy = create_run(&store, "agent-role-deploy").await;
    let release = create_run(&store, "agent-role-release").await;

    let first = create_step(
        &store,
        deploy,
        "review",
        StepKind::Agent,
        agent_input(Some(&role)),
    )
    .await;
    create_step(
        &store,
        deploy,
        "write",
        StepKind::Agent,
        agent_input(Some(&other)),
    )
    .await;
    let second = create_step(
        &store,
        release,
        "check",
        StepKind::Agent,
        agent_input(Some(&role)),
    )
    .await;

    let page = store
        .list_steps_by_agent_role(&role, 1, 20)
        .await
        .expect("list steps by role");
    assert_eq!(page.total, 2);
    let ids: Vec<Uuid> = page.items.iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![second.id, first.id], "newest first");
    assert!(
        page.items
            .iter()
            .all(|s| s.agent_role.as_deref() == Some(role.as_str()))
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn steps_by_role_paginate() {
    let store = get_store().await;
    let role = unique_role("pager");
    let run_id = create_run(&store, "agent-role").await;
    let mut ids = Vec::new();
    for i in 0..5 {
        let step = create_step(
            &store,
            run_id,
            &format!("review-{i}"),
            StepKind::Agent,
            agent_input(Some(&role)),
        )
        .await;
        ids.push(step.id);
    }
    ids.reverse();

    let first = store
        .list_steps_by_agent_role(&role, 1, 2)
        .await
        .expect("first page");
    assert_eq!(first.total, 5);
    let first_ids: Vec<Uuid> = first.items.iter().map(|s| s.id).collect();
    assert_eq!(first_ids, ids[0..2].to_vec());

    let last = store
        .list_steps_by_agent_role(&role, 3, 2)
        .await
        .expect("last page");
    let last_ids: Vec<Uuid> = last.items.iter().map(|s| s.id).collect();
    assert_eq!(last_ids, ids[4..5].to_vec());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn unknown_role_lists_nothing() {
    let store = get_store().await;
    let page = store
        .list_steps_by_agent_role(&unique_role("nobody"), 1, 20)
        .await
        .expect("list steps by role");
    assert_eq!(page.total, 0);
    assert!(page.items.is_empty());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn stats_group_cost_executions_and_failures_by_role() {
    let store = get_store().await;
    let reviewer = unique_role("stats-reviewer");
    let writer = unique_role("stats-writer");
    let deploy = create_run(&store, "agent-role-deploy").await;
    let release = create_run(&store, "agent-role-release").await;

    let a = create_step(
        &store,
        deploy,
        "review",
        StepKind::Agent,
        agent_input(Some(&reviewer)),
    )
    .await;
    finish_step(
        &store,
        a.id,
        StepStatus::Completed,
        Decimal::new(150, 2),
        1000,
    )
    .await;
    let b = create_step(
        &store,
        release,
        "review",
        StepKind::Agent,
        agent_input(Some(&reviewer)),
    )
    .await;
    finish_step(&store, b.id, StepStatus::Failed, Decimal::new(50, 2), 500).await;
    let c = create_step(
        &store,
        deploy,
        "write",
        StepKind::Agent,
        agent_input(Some(&writer)),
    )
    .await;
    finish_step(
        &store,
        c.id,
        StepStatus::Completed,
        Decimal::new(300, 2),
        4000,
    )
    .await;

    let stats = store
        .get_agent_role_stats(Utc::now() - Duration::days(1))
        .await
        .expect("agent role stats");

    assert_eq!(
        stats_of(&stats, &reviewer),
        Some(&AgentRoleStats {
            agent_role: reviewer.clone(),
            executions: 2,
            failed: 1,
            total_cost_usd: Decimal::new(200, 2),
            total_duration_ms: 1500,
        })
    );
    assert_eq!(
        stats_of(&stats, &writer),
        Some(&AgentRoleStats {
            agent_role: writer.clone(),
            executions: 1,
            failed: 0,
            total_cost_usd: Decimal::new(300, 2),
            total_duration_ms: 4000,
        })
    );
    for pair in stats.windows(2) {
        assert!(
            pair[0].total_cost_usd >= pair[1].total_cost_usd,
            "sorted by cost descending"
        );
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn stats_exclude_steps_before_created_after() {
    let store = get_store().await;
    let role = unique_role("stats-cutoff");
    let run_id = create_run(&store, "agent-role").await;
    let step = create_step(
        &store,
        run_id,
        "review",
        StepKind::Agent,
        agent_input(Some(&role)),
    )
    .await;

    let included = store
        .get_agent_role_stats(step.created_at)
        .await
        .expect("agent role stats");
    assert_eq!(
        stats_of(&included, &role).map(|s| s.executions),
        Some(1),
        "created_after is inclusive"
    );

    let excluded = store
        .get_agent_role_stats(step.created_at + Duration::seconds(1))
        .await
        .expect("agent role stats");
    assert_eq!(stats_of(&excluded, &role), None);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn stats_ignore_steps_without_role() {
    let store = get_store().await;
    let run_id = create_run(&store, "agent-role").await;
    let plain = create_step(&store, run_id, "plain", StepKind::Agent, agent_input(None)).await;
    finish_step(
        &store,
        plain.id,
        StepStatus::Completed,
        Decimal::new(100, 2),
        100,
    )
    .await;

    let stats = store
        .get_agent_role_stats(Utc::now() - Duration::days(1))
        .await
        .expect("agent role stats");
    assert!(stats.iter().all(|s| !s.agent_role.is_empty()));
}
