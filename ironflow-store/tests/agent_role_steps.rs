//! Integration tests for the agent role of a step on the in-memory store.
//!
//! The engine persists the agent config as the step input; a config built
//! from an `AgentRole` carries the role name under `role`. The store reads it
//! back as `Step::agent_role`, lists the steps of a role across runs and
//! workflows, and aggregates their cost, executions and failures.

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::time::sleep;
use uuid::Uuid;

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
        priority: 0,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
        worker_tags: Vec::new(),
    }
}

/// Input of an agent step, as the engine persists it.
fn agent_input(role: Option<&str>) -> Value {
    match role {
        Some(role) => json!({"type": "agent", "prompt": "p", "role": role}),
        None => json!({"type": "agent", "prompt": "p"}),
    }
}

async fn create_run(store: &InMemoryStore, workflow: &str) -> Uuid {
    store
        .create_run(new_run(workflow))
        .await
        .expect("create run")
        .into_run()
        .id
}

async fn create_step(
    store: &InMemoryStore,
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
    store: &InMemoryStore,
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

#[tokio::test]
async fn a_role_step_reads_back_its_role() {
    let store = InMemoryStore::new();
    let run_id = create_run(&store, "review").await;
    let created = create_step(
        &store,
        run_id,
        "review",
        StepKind::Agent,
        agent_input(Some("Reviewer")),
    )
    .await;
    assert_eq!(created.agent_role.as_deref(), Some("Reviewer"));

    let fetched = store
        .get_step(created.id)
        .await
        .expect("get step")
        .expect("step exists");
    assert_eq!(fetched.agent_role.as_deref(), Some("Reviewer"));

    let listed = store.list_steps(run_id).await.expect("list steps");
    assert_eq!(listed[0].agent_role.as_deref(), Some("Reviewer"));
}

#[tokio::test]
async fn an_agent_step_without_role_has_none() {
    let store = InMemoryStore::new();
    let run_id = create_run(&store, "review").await;
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
async fn steps_by_role_span_runs_and_workflows() {
    let store = InMemoryStore::new();
    let deploy = create_run(&store, "deploy").await;
    let release = create_run(&store, "release").await;

    let first = create_step(
        &store,
        deploy,
        "review",
        StepKind::Agent,
        agent_input(Some("Reviewer")),
    )
    .await;
    create_step(
        &store,
        deploy,
        "write",
        StepKind::Agent,
        agent_input(Some("Writer")),
    )
    .await;
    create_step(&store, release, "plain", StepKind::Agent, agent_input(None)).await;
    let second = create_step(
        &store,
        release,
        "check",
        StepKind::Agent,
        agent_input(Some("Reviewer")),
    )
    .await;

    let page = store
        .list_steps_by_agent_role("Reviewer", 1, 20)
        .await
        .expect("list steps by role");
    assert_eq!(page.total, 2);
    let ids: Vec<Uuid> = page.items.iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![second.id, first.id], "newest first");
    assert!(
        page.items
            .iter()
            .all(|s| s.agent_role.as_deref() == Some("Reviewer"))
    );
}

#[tokio::test]
async fn steps_by_role_paginate() {
    let store = InMemoryStore::new();
    let run_id = create_run(&store, "review").await;
    let mut ids = Vec::new();
    for i in 0..5 {
        let step = create_step(
            &store,
            run_id,
            &format!("review-{i}"),
            StepKind::Agent,
            agent_input(Some("Reviewer")),
        )
        .await;
        ids.push(step.id);
    }
    ids.reverse();

    let first = store
        .list_steps_by_agent_role("Reviewer", 1, 2)
        .await
        .expect("first page");
    assert_eq!(first.total, 5);
    assert_eq!(first.page, 1);
    assert_eq!(first.per_page, 2);
    let first_ids: Vec<Uuid> = first.items.iter().map(|s| s.id).collect();
    assert_eq!(first_ids, ids[0..2].to_vec());

    let last = store
        .list_steps_by_agent_role("Reviewer", 3, 2)
        .await
        .expect("last page");
    let last_ids: Vec<Uuid> = last.items.iter().map(|s| s.id).collect();
    assert_eq!(last_ids, ids[4..5].to_vec());

    let zero = store
        .list_steps_by_agent_role("Reviewer", 0, 2)
        .await
        .expect("page 0");
    assert_eq!(zero.page, 1, "page 0 is the first page");
    assert_eq!(zero.items.len(), 2);
}

#[tokio::test]
async fn unknown_role_lists_nothing() {
    let store = InMemoryStore::new();
    let run_id = create_run(&store, "review").await;
    create_step(
        &store,
        run_id,
        "review",
        StepKind::Agent,
        agent_input(Some("Reviewer")),
    )
    .await;

    let page = store
        .list_steps_by_agent_role("reviewer", 1, 20)
        .await
        .expect("list steps by role");
    assert_eq!(page.total, 0, "the name matches exactly");
    assert!(page.items.is_empty());
}

#[tokio::test]
async fn steps_by_role_with_a_huge_page_do_not_overflow() {
    let store = InMemoryStore::new();
    let run_id = create_run(&store, "review").await;
    create_step(
        &store,
        run_id,
        "review",
        StepKind::Agent,
        agent_input(Some("Reviewer")),
    )
    .await;

    // (50_000_000 - 1) * 100 does not fit in a u32.
    let page = store
        .list_steps_by_agent_role("Reviewer", 50_000_000, 100)
        .await
        .expect("list steps by role");
    assert_eq!(page.total, 1);
    assert!(page.items.is_empty());
    assert_eq!(page.page, 50_000_000);
}

#[tokio::test]
async fn stats_group_cost_executions_and_failures_by_role() {
    let store = InMemoryStore::new();
    let deploy = create_run(&store, "deploy").await;
    let release = create_run(&store, "release").await;

    let a = create_step(
        &store,
        deploy,
        "review",
        StepKind::Agent,
        agent_input(Some("Reviewer")),
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
        agent_input(Some("Reviewer")),
    )
    .await;
    finish_step(&store, b.id, StepStatus::Failed, Decimal::new(50, 2), 500).await;
    let c = create_step(
        &store,
        deploy,
        "write",
        StepKind::Agent,
        agent_input(Some("Writer")),
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
    let d = create_step(
        &store,
        release,
        "audit",
        StepKind::Agent,
        agent_input(Some("Auditor")),
    )
    .await;
    finish_step(
        &store,
        d.id,
        StepStatus::Completed,
        Decimal::new(200, 2),
        10,
    )
    .await;

    let stats = store
        .get_agent_role_stats(Utc::now() - Duration::days(7))
        .await
        .expect("agent role stats");

    assert_eq!(
        stats,
        vec![
            AgentRoleStats {
                agent_role: "Writer".to_string(),
                executions: 1,
                failed: 0,
                total_cost_usd: Decimal::new(300, 2),
                total_duration_ms: 4000,
            },
            AgentRoleStats {
                agent_role: "Auditor".to_string(),
                executions: 1,
                failed: 0,
                total_cost_usd: Decimal::new(200, 2),
                total_duration_ms: 10,
            },
            AgentRoleStats {
                agent_role: "Reviewer".to_string(),
                executions: 2,
                failed: 1,
                total_cost_usd: Decimal::new(200, 2),
                total_duration_ms: 1500,
            },
        ],
        "sorted by cost descending, then by role name"
    );
}

#[tokio::test]
async fn stats_exclude_steps_before_created_after() {
    let store = InMemoryStore::new();
    let run_id = create_run(&store, "review").await;
    let old = create_step(
        &store,
        run_id,
        "old",
        StepKind::Agent,
        agent_input(Some("Reviewer")),
    )
    .await;

    sleep(StdDuration::from_millis(5)).await;
    let cutoff = old.created_at + Duration::milliseconds(1);
    let recent = create_step(
        &store,
        run_id,
        "recent",
        StepKind::Agent,
        agent_input(Some("Reviewer")),
    )
    .await;
    assert!(recent.created_at >= cutoff, "the second step is newer");

    let stats = store
        .get_agent_role_stats(cutoff)
        .await
        .expect("agent role stats");
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].executions, 1);

    let future = store
        .get_agent_role_stats(Utc::now() + Duration::days(1))
        .await
        .expect("agent role stats");
    assert!(future.is_empty());
}

#[tokio::test]
async fn stats_ignore_steps_without_role() {
    let store = InMemoryStore::new();
    let run_id = create_run(&store, "review").await;
    let plain = create_step(&store, run_id, "plain", StepKind::Agent, agent_input(None)).await;
    finish_step(
        &store,
        plain.id,
        StepStatus::Completed,
        Decimal::new(100, 2),
        100,
    )
    .await;
    create_step(
        &store,
        run_id,
        "ls",
        StepKind::Shell,
        json!({"type": "shell", "command": "ls", "role": "Reviewer"}),
    )
    .await;

    let stats = store
        .get_agent_role_stats(Utc::now() - Duration::days(1))
        .await
        .expect("agent role stats");
    assert!(stats.is_empty());
}
