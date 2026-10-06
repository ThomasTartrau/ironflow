//! Integration tests for InMemoryStore covering all RunStore operations.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use ironflow_store::prelude::*;
use rust_decimal::Decimal;
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
        concurrency_key: None,
        priority: 0,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
    }
}

fn new_step(run_id: Uuid, name: &str, position: u32) -> NewStep {
    NewStep {
        run_id,
        trace_id: step_trace_id(run_id, name, position),
        name: name.to_string(),
        kind: StepKind::Shell,
        position,
        input: None,
        is_error_handler: false,
    }
}

// ─── CRUD Operations ────────────────────────────────────────────

#[tokio::test]
async fn create_and_retrieve_run() {
    let store = InMemoryStore::new();
    let created = store
        .create_run(new_run("test-workflow"))
        .await
        .unwrap()
        .into_run();

    let retrieved = store.get_run(created.id).await.unwrap();
    assert!(retrieved.is_some());

    let run = retrieved.unwrap();
    assert_eq!(run.id, created.id);
    assert_eq!(run.workflow_name, "test-workflow");
    assert_eq!(run.status.state, RunStatus::Pending);
    assert_eq!(run.trigger, TriggerKind::Manual);
    assert_eq!(run.retry_count, 0);
    assert_eq!(run.max_retries, 3);
    assert_eq!(run.cost_usd, Decimal::ZERO);
    assert_eq!(run.duration_ms, 0);
    assert!(run.error.is_none());
    assert!(run.started_at.is_none());
    assert!(run.completed_at.is_none());
}

#[tokio::test]
async fn retrieve_nonexistent_run_returns_none() {
    let store = InMemoryStore::new();
    let result = store.get_run(Uuid::nil()).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn create_multiple_runs_with_unique_ids() {
    let store = InMemoryStore::new();
    let r1 = store.create_run(new_run("wf1")).await.unwrap().into_run();
    let r2 = store.create_run(new_run("wf2")).await.unwrap().into_run();
    let r3 = store.create_run(new_run("wf3")).await.unwrap().into_run();

    assert_ne!(r1.id, r2.id);
    assert_ne!(r2.id, r3.id);
    assert_ne!(r1.id, r3.id);
}

// ─── Filtering & Pagination ─────────────────────────────────────

#[tokio::test]
async fn list_runs_filters_by_workflow_name() {
    let store = InMemoryStore::new();
    store
        .create_run(new_run("deploy"))
        .await
        .unwrap()
        .into_run();
    store.create_run(new_run("test")).await.unwrap().into_run();
    store
        .create_run(new_run("deploy"))
        .await
        .unwrap()
        .into_run();
    store.create_run(new_run("build")).await.unwrap().into_run();

    let filter = RunFilter {
        workflow_name: Some("deploy".to_string()),
        ..RunFilter::default()
    };
    let page = store.list_runs(filter, 1, 100).await.unwrap();

    assert_eq!(page.total, 2);
    assert_eq!(page.items.len(), 2);
    assert!(page.items.iter().all(|r| r.workflow_name == "deploy"));
}

#[tokio::test]
async fn list_runs_filters_by_status() {
    let store = InMemoryStore::new();
    let r1 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let r2 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let _r3 = store.create_run(new_run("wf")).await.unwrap().into_run();

    // r1: Pending → Running
    store
        .update_run_status(r1.id, RunStatus::Running)
        .await
        .unwrap();

    // r2: Pending → Running → Completed
    store
        .update_run_status(r2.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(r2.id, RunStatus::Completed)
        .await
        .unwrap();

    let filter = RunFilter {
        status: Some(RunStatus::Running),
        ..RunFilter::default()
    };
    let page = store.list_runs(filter, 1, 100).await.unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].id, r1.id);
}

#[tokio::test]
async fn list_runs_filters_by_created_after() {
    let store = InMemoryStore::new();
    let before_time = Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let _r1 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let after_time = Utc::now();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let _r2 = store.create_run(new_run("wf")).await.unwrap().into_run();

    let filter = RunFilter {
        created_after: Some(before_time),
        ..RunFilter::default()
    };
    let page = store.list_runs(filter, 1, 100).await.unwrap();

    // r1 and r2 both created after before_time
    assert!(page.total >= 1);

    let filter = RunFilter {
        created_after: Some(after_time),
        ..RunFilter::default()
    };
    let page = store.list_runs(filter, 1, 100).await.unwrap();

    // Only r2 was created after after_time
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn list_runs_pagination_respects_page_size() {
    let store = InMemoryStore::new();

    // Create 10 runs
    for i in 0..10 {
        store
            .create_run(new_run(&format!("wf-{i}")))
            .await
            .unwrap()
            .into_run();
    }

    // Page 1: 3 items per page
    let page1 = store.list_runs(RunFilter::default(), 1, 3).await.unwrap();
    assert_eq!(page1.total, 10);
    assert_eq!(page1.page, 1);
    assert_eq!(page1.per_page, 3);
    assert_eq!(page1.items.len(), 3);

    // Page 2: 3 items per page
    let page2 = store.list_runs(RunFilter::default(), 2, 3).await.unwrap();
    assert_eq!(page2.page, 2);
    assert_eq!(page2.items.len(), 3);

    // Verify no overlap
    let ids1: std::collections::HashSet<_> = page1.items.iter().map(|r| r.id).collect();
    let ids2: std::collections::HashSet<_> = page2.items.iter().map(|r| r.id).collect();
    assert!(ids1.is_disjoint(&ids2));
}

#[tokio::test]
async fn list_runs_page_beyond_end_returns_empty() {
    let store = InMemoryStore::new();
    store.create_run(new_run("wf")).await.unwrap().into_run();

    let page = store
        .list_runs(RunFilter::default(), 100, 10)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 0);
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn list_runs_ordered_by_created_at_descending() {
    let store = InMemoryStore::new();
    let r1 = store.create_run(new_run("wf1")).await.unwrap().into_run();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let r2 = store.create_run(new_run("wf2")).await.unwrap().into_run();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let r3 = store.create_run(new_run("wf3")).await.unwrap().into_run();

    let page = store.list_runs(RunFilter::default(), 1, 100).await.unwrap();
    assert_eq!(page.items.len(), 3);

    // Should be newest first: r3, r2, r1
    assert_eq!(page.items[0].id, r3.id);
    assert_eq!(page.items[1].id, r2.id);
    assert_eq!(page.items[2].id, r1.id);
}

// ─── Status Transitions ──────────────────────────────────────────

#[tokio::test]
async fn update_run_status_valid_transition_sets_timestamps() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    assert!(run.started_at.is_none());
    assert!(run.completed_at.is_none());

    // Pending → Running
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();

    let run = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Running);
    assert!(run.started_at.is_some());
    assert!(run.completed_at.is_none());

    // Running → Completed (terminal)
    store
        .update_run_status(run.id, RunStatus::Completed)
        .await
        .unwrap();

    let run = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Completed);
    assert!(run.started_at.is_some());
    assert!(run.completed_at.is_some());
}

#[tokio::test]
async fn update_run_status_invalid_transition_errors() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    // Pending → Completed is invalid
    let result = store.update_run_status(run.id, RunStatus::Completed).await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(matches!(err, StoreError::InvalidTransition { .. }));
}

#[tokio::test]
async fn update_run_status_terminal_state_to_terminal_errors() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    // Pending → Completed
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(run.id, RunStatus::Completed)
        .await
        .unwrap();

    // Completed → Running is invalid
    let result = store.update_run_status(run.id, RunStatus::Running).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn update_run_status_nonexistent_run_errors() {
    let store = InMemoryStore::new();
    let result = store
        .update_run_status(Uuid::nil(), RunStatus::Running)
        .await;
    assert!(matches!(result.unwrap_err(), StoreError::RunNotFound(_)));
}

// ─── Partial Updates ─────────────────────────────────────────────

#[tokio::test]
async fn update_run_applies_cost_duration_and_error() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    let cost = Decimal::new(12345, 2);
    store
        .update_run(
            run.id,
            RunUpdate {
                cost_usd: Some(cost),
                duration_ms: Some(5000),
                error: Some("test error".to_string()),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    let run = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(run.cost_usd, cost);
    assert_eq!(run.duration_ms, 5000);
    assert_eq!(run.error, Some("test error".to_string()));
}

#[tokio::test]
async fn update_run_increment_retry_increments_count() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();
    assert_eq!(run.retry_count, 0);

    store
        .update_run(
            run.id,
            RunUpdate {
                increment_retry: true,
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    let run = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(run.retry_count, 1);

    store
        .update_run(
            run.id,
            RunUpdate {
                increment_retry: true,
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    let run = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(run.retry_count, 2);
}

#[tokio::test]
async fn update_run_nonexistent_run_errors() {
    let store = InMemoryStore::new();
    let result = store
        .update_run(
            Uuid::nil(),
            RunUpdate {
                cost_usd: Some(Decimal::ZERO),
                ..RunUpdate::default()
            },
        )
        .await;
    assert!(matches!(result.unwrap_err(), StoreError::RunNotFound(_)));
}

// ─── Pick Next Pending ──────────────────────────────────────────

#[tokio::test]
async fn pick_next_pending_empty_store_returns_none() {
    let store = InMemoryStore::new();
    let result = store.pick_next_pending(None).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn pick_next_pending_returns_oldest_pending() {
    let store = InMemoryStore::new();
    let r1 = store.create_run(new_run("wf1")).await.unwrap().into_run();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let _r2 = store.create_run(new_run("wf2")).await.unwrap().into_run();

    let picked = store.pick_next_pending(None).await.unwrap().unwrap();
    assert_eq!(picked.id, r1.id);
    assert_eq!(picked.status.state, RunStatus::Running);
}

#[tokio::test]
async fn pick_next_pending_transitions_to_running() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();
    assert_eq!(run.status.state, RunStatus::Pending);

    let picked = store.pick_next_pending(None).await.unwrap().unwrap();
    assert_eq!(picked.status.state, RunStatus::Running);
    assert!(picked.started_at.is_some());

    // Verify in store as well
    let fetched = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(fetched.status.state, RunStatus::Running);
}

#[tokio::test]
async fn pick_next_pending_skips_non_pending_runs() {
    let store = InMemoryStore::new();
    let r1 = store.create_run(new_run("wf1")).await.unwrap().into_run();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let r2 = store.create_run(new_run("wf2")).await.unwrap().into_run();

    // Transition r1 to Running
    store
        .update_run_status(r1.id, RunStatus::Running)
        .await
        .unwrap();

    // Should pick r2 (the next oldest pending)
    let picked = store.pick_next_pending(None).await.unwrap().unwrap();
    assert_eq!(picked.id, r2.id);
}

// ─── Steps ──────────────────────────────────────────────────────

#[tokio::test]
async fn create_step_for_existing_run() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    let step = store
        .create_step(new_step(run.id, "build", 0))
        .await
        .unwrap();

    assert_eq!(step.run_id, run.id);
    assert_eq!(step.name, "build");
    assert_eq!(step.position, 0);
    assert_eq!(step.kind, StepKind::Shell);
    assert_eq!(step.status.state, StepStatus::Pending);
    assert_eq!(step.duration_ms, 0);
    assert_eq!(step.cost_usd, Decimal::ZERO);
    assert!(step.input.is_none());
    assert!(step.output.is_none());
    assert!(step.error.is_none());
}

#[tokio::test]
async fn create_step_for_nonexistent_run_errors() {
    let store = InMemoryStore::new();
    let result = store.create_step(new_step(Uuid::nil(), "build", 0)).await;
    assert!(matches!(result.unwrap_err(), StoreError::RunNotFound(_)));
}

#[tokio::test]
async fn list_steps_returns_steps_ordered_by_position() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    // Insert out of order
    store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "step3", 2),
            name: "step3".to_string(),
            kind: StepKind::Shell,
            position: 2,
            input: None,
            is_error_handler: false,
        })
        .await
        .unwrap();

    store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "step1", 0),
            name: "step1".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .unwrap();

    store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "step2", 1),
            name: "step2".to_string(),
            kind: StepKind::Shell,
            position: 1,
            input: None,
            is_error_handler: false,
        })
        .await
        .unwrap();

    let steps = store.list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].name, "step1");
    assert_eq!(steps[1].name, "step2");
    assert_eq!(steps[2].name, "step3");
}

#[tokio::test]
async fn list_steps_empty_for_run_with_no_steps() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    let steps = store.list_steps(run.id).await.unwrap();
    assert!(steps.is_empty());
}

#[tokio::test]
async fn list_steps_filters_by_run_id() {
    let store = InMemoryStore::new();
    let run1 = store.create_run(new_run("wf1")).await.unwrap().into_run();
    let run2 = store.create_run(new_run("wf2")).await.unwrap().into_run();

    store
        .create_step(new_step(run1.id, "step1", 0))
        .await
        .unwrap();
    store
        .create_step(new_step(run1.id, "step2", 1))
        .await
        .unwrap();
    store
        .create_step(new_step(run2.id, "step3", 0))
        .await
        .unwrap();

    let steps1 = store.list_steps(run1.id).await.unwrap();
    assert_eq!(steps1.len(), 2);
    assert!(steps1.iter().all(|s| s.run_id == run1.id));

    let steps2 = store.list_steps(run2.id).await.unwrap();
    assert_eq!(steps2.len(), 1);
    assert_eq!(steps2[0].run_id, run2.id);
}

#[tokio::test]
async fn update_step_applies_partial_updates() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();
    let step = store
        .create_step(new_step(run.id, "build", 0))
        .await
        .unwrap();

    // Pending → Running
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();

    // Running → Completed with output
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                output: Some(json!({"result": "ok"})),
                duration_ms: Some(1500),
                cost_usd: Some(Decimal::new(50, 2)),
                input_tokens: Some(100),
                cache_read_input_tokens: Some(1000),
                cache_creation_input_tokens: Some(50),
                output_tokens: Some(200),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();

    let steps = store.list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 1);
    let step = &steps[0];

    assert_eq!(step.status.state, StepStatus::Completed);
    assert_eq!(step.output, Some(json!({"result": "ok"})));
    assert_eq!(step.duration_ms, 1500);
    assert_eq!(step.cost_usd, Decimal::new(50, 2));
    assert_eq!(step.input_tokens, Some(100));
    assert_eq!(step.cache_read_input_tokens, Some(1000));
    assert_eq!(step.cache_creation_input_tokens, Some(50));
    assert_eq!(step.output_tokens, Some(200));
}

#[tokio::test]
async fn update_step_nonexistent_step_errors() {
    let store = InMemoryStore::new();
    let result = store
        .update_step(
            Uuid::nil(),
            StepUpdate {
                status: Some(StepStatus::Completed),
                ..StepUpdate::default()
            },
        )
        .await;
    assert!(matches!(result.unwrap_err(), StoreError::StepNotFound(_)));
}

// ─── Statistics ──────────────────────────────────────────────────

#[tokio::test]
async fn get_stats_empty_store() {
    let store = InMemoryStore::new();
    let stats = store.get_stats(RunFilter::default()).await.unwrap();

    assert_eq!(stats.total_runs, 0);
    assert_eq!(stats.completed_runs, 0);
    assert_eq!(stats.failed_runs, 0);
    assert_eq!(stats.cancelled_runs, 0);
    assert_eq!(stats.active_runs, 0);
    assert_eq!(stats.total_cost_usd, Decimal::ZERO);
    assert_eq!(stats.total_duration_ms, 0);
}

#[tokio::test]
async fn get_stats_aggregates_by_status() {
    let store = InMemoryStore::new();

    let r1 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let r2 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let r3 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let _r4 = store.create_run(new_run("wf")).await.unwrap().into_run();

    // r1: Completed
    store
        .update_run_status(r1.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(r1.id, RunStatus::Completed)
        .await
        .unwrap();

    // r2: Failed
    store
        .update_run_status(r2.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(r2.id, RunStatus::Failed)
        .await
        .unwrap();

    // r3: Cancelled
    store
        .update_run_status(r3.id, RunStatus::Cancelled)
        .await
        .unwrap();

    // _r4: Pending (active)

    let stats = store.get_stats(RunFilter::default()).await.unwrap();
    assert_eq!(stats.total_runs, 4);
    assert_eq!(stats.completed_runs, 1);
    assert_eq!(stats.failed_runs, 1);
    assert_eq!(stats.cancelled_runs, 1);
    assert_eq!(stats.active_runs, 1); // _r4 is Pending
}

#[tokio::test]
async fn get_stats_aggregates_cost_and_duration() {
    let store = InMemoryStore::new();

    let r1 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let r2 = store.create_run(new_run("wf")).await.unwrap().into_run();

    store
        .update_run(
            r1.id,
            RunUpdate {
                cost_usd: Some(Decimal::new(10000, 2)),
                duration_ms: Some(3000),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    store
        .update_run(
            r2.id,
            RunUpdate {
                cost_usd: Some(Decimal::new(5000, 2)),
                duration_ms: Some(2000),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    let stats = store.get_stats(RunFilter::default()).await.unwrap();
    assert_eq!(stats.total_runs, 2);
    assert_eq!(stats.total_cost_usd, Decimal::new(15000, 2));
    assert_eq!(stats.total_duration_ms, 5000);
}

/// Regression test: PostgreSQL returns `NUMERIC` for `SUM(BIGINT)`.
/// Without an explicit `::BIGINT` cast, `row.get::<i64, _>()` panics at runtime.
/// This test ensures `total_duration_ms` handles values exceeding `i32::MAX`,
/// which is the scenario that originally exposed the type mismatch.
#[tokio::test]
async fn get_stats_total_duration_exceeds_i32_max() {
    let store = InMemoryStore::new();

    let r1 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let r2 = store.create_run(new_run("wf")).await.unwrap().into_run();

    // Each duration alone fits in i32, but their sum exceeds i32::MAX (2_147_483_647).
    let half: u64 = 1_200_000_000; // 1.2 billion ms (~333 hours)

    store
        .update_run(
            r1.id,
            RunUpdate {
                duration_ms: Some(half),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    store
        .update_run(
            r2.id,
            RunUpdate {
                duration_ms: Some(half),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    let stats = store.get_stats(RunFilter::default()).await.unwrap();
    assert_eq!(stats.total_duration_ms, half * 2);
    assert!(stats.total_duration_ms > i32::MAX as u64);
}

#[tokio::test]
async fn get_stats_active_runs_counts_pending_running_retrying() {
    let store = InMemoryStore::new();

    let r1 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let r2 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let r3 = store.create_run(new_run("wf")).await.unwrap().into_run();
    let _r4 = store.create_run(new_run("wf")).await.unwrap().into_run(); // Pending

    // r1: Pending → Running
    store
        .update_run_status(r1.id, RunStatus::Running)
        .await
        .unwrap();

    // r2: Pending → Running → Retrying
    store
        .update_run_status(r2.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(r2.id, RunStatus::Retrying)
        .await
        .unwrap();

    // r3: Pending → Running → Completed
    store
        .update_run_status(r3.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(r3.id, RunStatus::Completed)
        .await
        .unwrap();

    let stats = store.get_stats(RunFilter::default()).await.unwrap();
    assert_eq!(stats.active_runs, 3); // r1 (Running), r2 (Retrying), _r4 (Pending)
    assert_eq!(stats.completed_runs, 1); // r3
}

// ─── Direct Cancellation ────────────────────────────────────────

#[tokio::test]
async fn update_run_status_pending_to_cancelled() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    store
        .update_run_status(run.id, RunStatus::Cancelled)
        .await
        .unwrap();

    let run = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Cancelled);
    assert!(run.completed_at.is_some());
}

#[tokio::test]
async fn update_run_status_running_to_cancelled() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(run.id, RunStatus::Cancelled)
        .await
        .unwrap();

    let run = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Cancelled);
    assert!(run.started_at.is_some());
    assert!(run.completed_at.is_some());
}

// ─── Step Status Transitions ────────────────────────────────────

#[tokio::test]
async fn update_step_completed_to_running_errors() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();
    let step = store
        .create_step(new_step(run.id, "build", 0))
        .await
        .unwrap();

    // Pending → Running → Completed
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();

    // Completed → Running is invalid
    let result = store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                ..StepUpdate::default()
            },
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_steps_nonexistent_run_returns_empty() {
    let store = InMemoryStore::new();
    let steps = store.list_steps(Uuid::nil()).await.unwrap();
    assert!(steps.is_empty());
}

// ─── Edge Cases ──────────────────────────────────────────────────

#[tokio::test]
async fn pagination_edge_case_page_zero_defaults_to_one() {
    let store = InMemoryStore::new();
    store.create_run(new_run("wf")).await.unwrap().into_run();

    // Page 0 should be clamped to page 1
    let page = store.list_runs(RunFilter::default(), 0, 10).await.unwrap();
    assert_eq!(page.page, 1);
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn pagination_edge_case_per_page_zero_defaults_to_one() {
    let store = InMemoryStore::new();
    for i in 0..5 {
        store
            .create_run(new_run(&format!("wf-{i}")))
            .await
            .unwrap()
            .into_run();
    }

    // per_page 0 should be clamped to 1
    let page = store.list_runs(RunFilter::default(), 1, 0).await.unwrap();
    assert_eq!(page.per_page, 1);
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn pagination_edge_case_per_page_exceeds_max() {
    let store = InMemoryStore::new();
    for i in 0..5 {
        store
            .create_run(new_run(&format!("wf-{i}")))
            .await
            .unwrap()
            .into_run();
    }

    // per_page > 100 should be clamped to 100
    let page = store.list_runs(RunFilter::default(), 1, 200).await.unwrap();
    assert_eq!(page.per_page, 100);
    assert_eq!(page.items.len(), 5);
}

#[tokio::test]
async fn concurrent_creates_do_not_corrupt_store() {
    let store = InMemoryStore::new();
    let mut handles = Vec::new();

    for i in 0..10 {
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            s.create_run(new_run(&format!("wf-{i}")))
                .await
                .map(|creation| creation.into_run())
        }));
    }

    let mut created_ids = std::collections::HashSet::new();
    for h in handles {
        if let Ok(Ok(run)) = h.await {
            assert!(created_ids.insert(run.id), "duplicate run ID");
        }
    }

    assert_eq!(created_ids.len(), 10);

    // Verify all can be retrieved
    let stats = store.get_stats(RunFilter::default()).await.unwrap();
    assert_eq!(stats.total_runs, 10);
}

#[tokio::test]
async fn large_payload_preserved_in_roundtrip() {
    let store = InMemoryStore::new();

    let large_payload = json!({
        "nested": {
            "data": vec!["a", "b", "c"],
            "count": 1000,
            "unicode": "こんにちは🚀"
        }
    });

    let req = NewRun {
        created_by: None,
        workflow_name: "test".to_string(),
        trigger: TriggerKind::Manual,
        payload: large_payload.clone(),
        max_retries: 1,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        idempotency_key: None,
        concurrency_key: None,
        priority: 0,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
    };

    let run = store.create_run(req).await.unwrap().into_run();
    let retrieved = store.get_run(run.id).await.unwrap().unwrap();

    assert_eq!(retrieved.payload, large_payload);
}

// ---- idempotency key ----

fn new_run_with_key(name: &str, key: &str) -> NewRun {
    NewRun {
        idempotency_key: Some(key.to_string()),
        ..new_run(name)
    }
}

#[tokio::test]
async fn create_run_without_key_never_deduplicates() {
    let store = InMemoryStore::new();

    let first = store.create_run(new_run("deploy")).await.unwrap();
    let second = store.create_run(new_run("deploy")).await.unwrap();

    assert!(first.is_created());
    assert!(second.is_created());
    assert_ne!(first.run().id, second.run().id);
}

#[tokio::test]
async fn create_run_with_key_binds_the_key_to_the_run() {
    let store = InMemoryStore::new();

    let creation = store
        .create_run(new_run_with_key("deploy", "github:abc-123"))
        .await
        .unwrap();

    assert!(creation.is_created());
    assert_eq!(
        creation.run().idempotency_key.as_deref(),
        Some("github:abc-123")
    );
}

#[tokio::test]
async fn create_run_replays_a_known_key() {
    let store = InMemoryStore::new();

    let first = store
        .create_run(new_run_with_key("deploy", "github:abc-123"))
        .await
        .unwrap();
    let second = store
        .create_run(new_run_with_key("deploy", "github:abc-123"))
        .await
        .unwrap();

    assert!(first.is_created());
    assert!(!second.is_created());
    assert_eq!(first.run().id, second.run().id);
}

#[tokio::test]
async fn create_run_isolates_distinct_keys() {
    let store = InMemoryStore::new();

    let first = store
        .create_run(new_run_with_key("deploy", "github:abc"))
        .await
        .unwrap();
    let second = store
        .create_run(new_run_with_key("deploy", "github:def"))
        .await
        .unwrap();

    assert!(second.is_created());
    assert_ne!(first.run().id, second.run().id);
}

#[tokio::test]
async fn create_run_replays_across_different_workflows() {
    let store = InMemoryStore::new();

    let first = store
        .create_run(new_run_with_key("deploy", "shared-key"))
        .await
        .unwrap();
    let second = store
        .create_run(new_run_with_key("rollback", "shared-key"))
        .await
        .unwrap();

    // The key is global, not scoped per workflow.
    assert!(!second.is_created());
    assert_eq!(first.run().id, second.run().id);
    assert_eq!(second.run().workflow_name, "deploy");
}

#[tokio::test]
async fn create_run_replays_a_key_bound_to_a_terminal_run() {
    let store = InMemoryStore::new();

    let first = store
        .create_run(new_run_with_key("deploy", "github:abc"))
        .await
        .unwrap()
        .into_run();
    store
        .update_run_status(first.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status(first.id, RunStatus::Failed)
        .await
        .unwrap();

    let replay = store
        .create_run(new_run_with_key("deploy", "github:abc"))
        .await
        .unwrap();

    assert!(!replay.is_created());
    assert_eq!(replay.run().id, first.id);
    assert_eq!(replay.run().status.state, RunStatus::Failed);
}

#[tokio::test]
async fn find_run_by_idempotency_key_returns_the_bound_run() {
    let store = InMemoryStore::new();

    let created = store
        .create_run(new_run_with_key("deploy", "github:abc"))
        .await
        .unwrap()
        .into_run();

    let found = store
        .find_run_by_idempotency_key("github:abc")
        .await
        .unwrap();

    assert_eq!(found.expect("run bound to the key").id, created.id);
}

#[tokio::test]
async fn find_run_by_idempotency_key_returns_none_for_unknown_key() {
    let store = InMemoryStore::new();

    let found = store
        .find_run_by_idempotency_key("never-used")
        .await
        .unwrap();

    assert!(found.is_none());
}

#[tokio::test]
async fn find_run_by_idempotency_key_ignores_a_run_without_key() {
    let store = InMemoryStore::new();

    store.create_run(new_run("deploy")).await.unwrap();

    let found = store.find_run_by_idempotency_key("").await.unwrap();

    assert!(found.is_none());
}

#[tokio::test]
async fn concurrent_creates_with_the_same_key_produce_one_run() {
    let store = InMemoryStore::new();
    let mut handles = Vec::new();

    for _ in 0..50 {
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            s.create_run(new_run_with_key("deploy", "github:race"))
                .await
                .unwrap()
        }));
    }

    let mut created = 0;
    let mut ids = std::collections::HashSet::new();
    for handle in handles {
        let creation = handle.await.unwrap();
        if creation.is_created() {
            created += 1;
        }
        ids.insert(creation.run().id);
    }

    assert_eq!(created, 1, "exactly one caller should create the run");
    assert_eq!(ids.len(), 1, "all callers should resolve to the same run");

    let page = store.list_runs(RunFilter::default(), 1, 100).await.unwrap();
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn concurrent_creates_with_distinct_keys_produce_distinct_runs() {
    let store = InMemoryStore::new();
    let mut handles = Vec::new();

    for i in 0..20 {
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            s.create_run(new_run_with_key("deploy", &format!("key-{i}")))
                .await
                .unwrap()
        }));
    }

    let mut ids = std::collections::HashSet::new();
    for handle in handles {
        let creation = handle.await.unwrap();
        assert!(creation.is_created());
        ids.insert(creation.run().id);
    }

    assert_eq!(ids.len(), 20);
}

#[tokio::test]
async fn unicode_key_is_stored_verbatim() {
    let store = InMemoryStore::new();

    let created = store
        .create_run(new_run_with_key("deploy", "clé-🚀"))
        .await
        .unwrap()
        .into_run();

    let found = store.find_run_by_idempotency_key("clé-🚀").await.unwrap();

    assert_eq!(found.expect("run bound to the key").id, created.id);
}

// ─── Provider Accounts ──────────────────────────────────────────

fn new_account(name: &str, priority: i32) -> NewProviderAccount {
    let id = Uuid::now_v7();
    NewProviderAccount {
        id,
        name: name.to_string(),
        display_name: name.to_uppercase(),
        kind: "claude_subscription".to_string(),
        secret_key: provider_account_secret_key(id),
        enabled: true,
        priority,
        tags: vec!["team".to_string()],
        max_concurrency: None,
        alert_threshold: 0.8,
        expires_at: Utc::now() + TimeDelta::days(365),
        plan: Some("max".to_string()),
        created_by: None,
    }
}

fn account_window(name: &str, utilization: f64, observed_at: DateTime<Utc>) -> NewAccountWindow {
    NewAccountWindow {
        window: name.to_string(),
        utilization,
        resets_at: Some(observed_at + TimeDelta::hours(2)),
        status: AccountWindowStatus::Allowed,
        model_scope: None,
        observed_at,
    }
}

#[tokio::test]
async fn provider_account_crud_and_duplicate_name() {
    let store = InMemoryStore::new();
    let created = store
        .create_provider_account(new_account("perso", 10))
        .await
        .unwrap();
    assert_eq!(created.name, "perso");
    assert!(created.auth_failed_at.is_none());

    let err = store
        .create_provider_account(new_account("perso", 20))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::DuplicateProviderAccount(ref n) if n == "perso"));

    let by_id = store.get_provider_account(created.id).await.unwrap();
    assert_eq!(by_id.as_ref().map(|a| a.id), Some(created.id));
    let by_name = store.find_provider_account_by_name("perso").await.unwrap();
    assert_eq!(by_name.map(|a| a.id), Some(created.id));
    assert!(
        store
            .find_provider_account_by_name("missing")
            .await
            .unwrap()
            .is_none()
    );

    store
        .create_provider_account(new_account("alpha", 5))
        .await
        .unwrap();
    let page = store.list_provider_accounts(None, 1, 20).await.unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.items[0].name, "alpha");
    let none = store
        .list_provider_accounts(Some("other".to_string()), 1, 20)
        .await
        .unwrap();
    assert_eq!(none.total, 0);

    let updated = store
        .update_provider_account(
            created.id,
            ProviderAccountUpdate {
                enabled: Some(false),
                max_concurrency: Some(Some(2)),
                plan: Some(None),
                ..ProviderAccountUpdate::default()
            },
        )
        .await
        .unwrap();
    assert!(!updated.enabled);
    assert_eq!(updated.max_concurrency, Some(2));
    assert_eq!(updated.plan, None);
    assert!(updated.updated_at >= created.updated_at);

    let missing = store
        .update_provider_account(Uuid::now_v7(), ProviderAccountUpdate::default())
        .await
        .unwrap_err();
    assert!(matches!(missing, StoreError::ProviderAccountNotFound(_)));

    assert!(store.delete_provider_account(created.id).await.unwrap());
    assert!(!store.delete_provider_account(created.id).await.unwrap());
}

#[tokio::test]
async fn provider_account_observation_upserts_and_writes_history() {
    let store = InMemoryStore::new();
    let account = store
        .create_provider_account(new_account("perso", 10))
        .await
        .unwrap();
    let t0 = Utc::now() - TimeDelta::minutes(10);
    let t1 = t0 + TimeDelta::minutes(5);

    store
        .record_provider_account_observation(
            account.id,
            NewProviderAccountObservation {
                windows: vec![
                    account_window("five_hour", 0.2, t0),
                    account_window("seven_day", 0.5, t0),
                ],
                auth_failed: false,
            },
        )
        .await
        .unwrap();
    let windows = store
        .record_provider_account_observation(
            account.id,
            NewProviderAccountObservation {
                windows: vec![account_window("five_hour", 0.4, t1)],
                auth_failed: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(windows.len(), 2);
    let five = windows.iter().find(|w| w.window == "five_hour").unwrap();
    assert!((five.utilization - 0.4).abs() < 1e-9);

    let history = store
        .list_provider_account_usage(account.id, t0 - TimeDelta::minutes(1))
        .await
        .unwrap();
    assert_eq!(history.len(), 3);
    assert!(
        history
            .windows(2)
            .all(|p| p[0].observed_at <= p[1].observed_at)
    );

    let missing = store
        .record_provider_account_observation(
            Uuid::now_v7(),
            NewProviderAccountObservation::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(missing, StoreError::ProviderAccountNotFound(_)));
}

#[tokio::test]
async fn provider_account_older_observation_does_not_overwrite_newer() {
    let store = InMemoryStore::new();
    let account = store
        .create_provider_account(new_account("perso", 10))
        .await
        .unwrap();
    let newer = Utc::now();
    let older = newer - TimeDelta::minutes(30);
    for (utilization, at) in [(0.7, newer), (0.1, older)] {
        store
            .record_provider_account_observation(
                account.id,
                NewProviderAccountObservation {
                    windows: vec![account_window("five_hour", utilization, at)],
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
    assert_eq!(windows[0].observed_at, newer);
}

#[tokio::test]
async fn provider_account_auth_failure_is_set_and_cleared() {
    let store = InMemoryStore::new();
    let account = store
        .create_provider_account(new_account("perso", 10))
        .await
        .unwrap();
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

    store
        .record_provider_account_observation(
            account.id,
            NewProviderAccountObservation {
                windows: vec![account_window("five_hour", 0.1, Utc::now())],
                auth_failed: false,
            },
        )
        .await
        .unwrap();
    let cleared = store
        .get_provider_account(account.id)
        .await
        .unwrap()
        .unwrap();
    assert!(cleared.auth_failed_at.is_none());
}

#[tokio::test]
async fn list_provider_accounts_by_ids_returns_only_existing_requested() {
    let store = InMemoryStore::new();
    let first = store
        .create_provider_account(new_account("first", 10))
        .await
        .unwrap();
    let second = store
        .create_provider_account(new_account("second", 10))
        .await
        .unwrap();
    store
        .create_provider_account(new_account("third", 10))
        .await
        .unwrap();

    let mut found = store
        .list_provider_accounts_by_ids(vec![first.id, second.id, Uuid::now_v7()])
        .await
        .unwrap();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<&str> = found.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, vec!["first", "second"]);

    assert!(
        store
            .list_provider_accounts_by_ids(Vec::new())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn provider_account_candidates_filter_and_count_running_steps() {
    let store = InMemoryStore::new();
    let active = store
        .create_provider_account(new_account("active", 10))
        .await
        .unwrap();
    let disabled = store
        .create_provider_account(NewProviderAccount {
            enabled: false,
            ..new_account("disabled", 10)
        })
        .await
        .unwrap();
    let expired = store
        .create_provider_account(NewProviderAccount {
            expires_at: Utc::now() - TimeDelta::days(1),
            ..new_account("expired", 10)
        })
        .await
        .unwrap();
    let failed = store
        .create_provider_account(new_account("failed", 10))
        .await
        .unwrap();
    store
        .record_provider_account_observation(
            failed.id,
            NewProviderAccountObservation {
                windows: Vec::new(),
                auth_failed: true,
            },
        )
        .await
        .unwrap();
    store
        .create_provider_account(NewProviderAccount {
            kind: "other".to_string(),
            ..new_account("other-kind", 10)
        })
        .await
        .unwrap();

    // A running step under `active`, on a run holding a live lease.
    store.create_run(new_run("wf")).await.unwrap();
    let run = store
        .pick_next_pending(Some(LeaseRequest {
            worker_id: "worker-1".to_string(),
            ttl: Duration::from_secs(90),
        }))
        .await
        .unwrap()
        .expect("picked run");
    let step = store
        .create_step(new_step(run.id, "agent", 0))
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
        .list_provider_account_candidates("claude_subscription".to_string())
        .await
        .unwrap();
    let names: Vec<&str> = candidates.iter().map(|c| c.account.name.as_str()).collect();
    assert_eq!(names, vec!["active"]);
    assert_eq!(candidates[0].running_steps, 1);
    assert!(!names.contains(&disabled.name.as_str()));
    assert!(!names.contains(&expired.name.as_str()));
}

#[tokio::test]
async fn provider_account_delete_cascades_and_nulls_step_account() {
    let store = InMemoryStore::new();
    let account = store
        .create_provider_account(new_account("perso", 10))
        .await
        .unwrap();
    store
        .record_provider_account_observation(
            account.id,
            NewProviderAccountObservation {
                windows: vec![account_window("five_hour", 0.3, Utc::now())],
                auth_failed: false,
            },
        )
        .await
        .unwrap();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();
    let step = store
        .create_step(new_step(run.id, "agent", 0))
        .await
        .unwrap();
    store
        .update_step(
            step.id,
            StepUpdate {
                account_id: Some(account.id),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store.get_step(step.id).await.unwrap().unwrap().account_id,
        Some(account.id)
    );

    assert!(store.delete_provider_account(account.id).await.unwrap());
    assert!(
        store
            .list_provider_account_windows(vec![account.id])
            .await
            .unwrap()
            .is_empty()
    );
    let since = Utc::now() - TimeDelta::days(1);
    assert!(
        store
            .list_provider_account_usage(account.id, since)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.get_step(step.id).await.unwrap().unwrap().account_id,
        None
    );
}

#[tokio::test]
async fn provider_account_purge_usage_removes_old_points() {
    let store = InMemoryStore::new();
    let account = store
        .create_provider_account(new_account("perso", 10))
        .await
        .unwrap();
    let now = Utc::now();
    store
        .record_provider_account_observation(
            account.id,
            NewProviderAccountObservation {
                windows: vec![
                    account_window("five_hour", 0.1, now - TimeDelta::days(40)),
                    account_window("seven_day", 0.2, now),
                ],
                auth_failed: false,
            },
        )
        .await
        .unwrap();
    let purged = store
        .purge_provider_account_usage(now - TimeDelta::days(30))
        .await
        .unwrap();
    assert_eq!(purged, 1);
    let remaining = store
        .list_provider_account_usage(account.id, now - TimeDelta::days(90))
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].window, "seven_day");
}

#[cfg(feature = "secret-store")]
#[tokio::test]
async fn list_secrets_hides_provider_account_credentials() {
    use ironflow_store::crypto::KeyRing;

    let mut store = InMemoryStore::new();
    let spec = format!("1:{}", "aa".repeat(32));
    store.set_key_ring(KeyRing::from_spec(&spec, Some(1)).unwrap());
    store.set_secret("github/token", "v").await.unwrap();
    store
        .set_secret(
            &provider_account_secret_key(Uuid::now_v7()),
            "sk-ant-oat01-x",
        )
        .await
        .unwrap();

    let page = store.list_secrets("", 1, 50).await.unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].key, "github/token");
    let hidden = store.list_secrets("accounts/", 1, 50).await.unwrap();
    assert_eq!(hidden.total, 0);
}

// ---- concurrency key ----

fn new_run_holding(name: &str, key: &str) -> NewRun {
    NewRun {
        concurrency_key: Some(key.to_string()),
        ..new_run(name)
    }
}

/// Assert that creating a run with `key` conflicts with `holder`.
async fn assert_conflicts_with(store: &InMemoryStore, key: &str, holder: Uuid) {
    match store.create_run(new_run_holding("deploy", key)).await {
        Err(StoreError::ConcurrencyConflict {
            key: conflict_key,
            run_id,
        }) => {
            assert_eq!(conflict_key, key);
            assert_eq!(run_id, holder);
        }
        other => panic!("expected a concurrency conflict, got {other:?}"),
    }
}

#[tokio::test]
async fn concurrent_creates_with_the_same_concurrency_key_create_one_run() {
    let store = InMemoryStore::new();
    let mut handles = Vec::new();

    for _ in 0..50 {
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            s.create_run(new_run_holding("deploy", "issue:12")).await
        }));
    }

    let mut created = Vec::new();
    let mut conflicts = Vec::new();
    for handle in handles {
        match handle.await.unwrap() {
            Ok(creation) => {
                assert!(creation.is_created());
                created.push(creation.into_run().id);
            }
            Err(StoreError::ConcurrencyConflict { key, run_id }) => {
                assert_eq!(key, "issue:12");
                conflicts.push(run_id);
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    assert_eq!(created.len(), 1, "exactly one caller should create the run");
    assert_eq!(conflicts.len(), 49);
    assert!(conflicts.iter().all(|id| *id == created[0]));

    let page = store.list_runs(RunFilter::default(), 1, 100).await.unwrap();
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn concurrency_key_is_released_when_the_run_completes() {
    let store = InMemoryStore::new();

    let first = store
        .create_run(new_run_holding("deploy", "issue:12"))
        .await
        .unwrap()
        .into_run();
    assert_eq!(first.concurrency_key.as_deref(), Some("issue:12"));

    assert_conflicts_with(&store, "issue:12", first.id).await;

    store
        .update_run_status(first.id, RunStatus::Running)
        .await
        .unwrap();
    assert_conflicts_with(&store, "issue:12", first.id).await;
    store
        .update_run_status(first.id, RunStatus::Completed)
        .await
        .unwrap();

    let second = store
        .create_run(new_run_holding("deploy", "issue:12"))
        .await
        .unwrap();
    assert!(second.is_created());
    assert_ne!(second.run().id, first.id);
}

#[tokio::test]
async fn concurrency_key_is_kept_while_the_run_sleeps_or_awaits_approval() {
    for paused in [
        RunStatus::AwaitingApproval,
        RunStatus::Sleeping,
        RunStatus::Retrying,
    ] {
        let store = InMemoryStore::new();
        let holder = store
            .create_run(new_run_holding("deploy", "issue:12"))
            .await
            .unwrap()
            .into_run();
        store
            .update_run_status(holder.id, RunStatus::Running)
            .await
            .unwrap();
        store.update_run_status(holder.id, paused).await.unwrap();

        assert_conflicts_with(&store, "issue:12", holder.id).await;
    }
}

#[tokio::test]
async fn concurrency_key_released_by_cancel_and_failure() {
    for terminal in [RunStatus::Cancelled, RunStatus::Failed, RunStatus::Warning] {
        let store = InMemoryStore::new();
        let holder = store
            .create_run(new_run_holding("deploy", "issue:12"))
            .await
            .unwrap()
            .into_run();
        store
            .update_run_status(holder.id, RunStatus::Running)
            .await
            .unwrap();
        store.update_run_status(holder.id, terminal).await.unwrap();

        let next = store
            .create_run(new_run_holding("deploy", "issue:12"))
            .await
            .unwrap();
        assert!(next.is_created(), "{terminal} should release the key");
    }

    // A pending run cancelled before it started releases the key too.
    let store = InMemoryStore::new();
    let pending = store
        .create_run(new_run_holding("deploy", "issue:12"))
        .await
        .unwrap()
        .into_run();
    store
        .update_run_status(pending.id, RunStatus::Cancelled)
        .await
        .unwrap();
    assert!(
        store
            .create_run(new_run_holding("deploy", "issue:12"))
            .await
            .unwrap()
            .is_created()
    );
}

#[tokio::test]
async fn distinct_concurrency_keys_do_not_conflict() {
    let store = InMemoryStore::new();

    let first = store
        .create_run(new_run_holding("deploy", "issue:12"))
        .await
        .unwrap();
    let second = store
        .create_run(new_run_holding("deploy", "issue:13"))
        .await
        .unwrap();
    let without_key = store.create_run(new_run("deploy")).await.unwrap();

    assert!(first.is_created());
    assert!(second.is_created());
    assert!(without_key.is_created());
}

#[tokio::test]
async fn idempotent_replay_with_a_concurrency_key_returns_the_existing_run() {
    let store = InMemoryStore::new();
    let req = NewRun {
        idempotency_key: Some("github:abc".to_string()),
        ..new_run_holding("deploy", "issue:12")
    };

    let first = store.create_run(req.clone()).await.unwrap();
    let replay = store.create_run(req).await.unwrap();

    assert!(first.is_created());
    assert!(!replay.is_created());
    assert_eq!(replay.run().id, first.run().id);
}

// ---- concurrency groups ----

fn new_run_in(name: &str, limits: &[(&str, u32)]) -> NewRun {
    NewRun {
        concurrency_limits: limits
            .iter()
            .map(|(group, limit)| ConcurrencyLimit::new(*group, *limit))
            .collect(),
        ..new_run(name)
    }
}

async fn create(store: &InMemoryStore, req: NewRun) -> Run {
    store.create_run(req).await.unwrap().into_run()
}

async fn pick_id(store: &InMemoryStore) -> Option<Uuid> {
    store.pick_next_pending(None).await.unwrap().map(|r| r.id)
}

#[tokio::test]
async fn pick_next_pending_holds_back_run_beyond_group_limit() {
    let store = InMemoryStore::new();
    let r1 = create(&store, new_run_in("deploy", &[("g", 2)])).await;
    let r2 = create(&store, new_run_in("deploy", &[("g", 2)])).await;
    let r3 = create(&store, new_run_in("deploy", &[("g", 2)])).await;

    assert_eq!(pick_id(&store).await, Some(r1.id));
    assert_eq!(pick_id(&store).await, Some(r2.id));
    assert_eq!(pick_id(&store).await, None, "group g is saturated");

    let still_pending = store.get_run(r3.id).await.unwrap().unwrap();
    assert_eq!(still_pending.status.state, RunStatus::Pending);

    store
        .update_run_status(r1.id, RunStatus::Completed)
        .await
        .unwrap();

    assert_eq!(pick_id(&store).await, Some(r3.id));
}

#[tokio::test]
async fn pick_next_pending_skips_blocked_group_for_other_group() {
    let store = InMemoryStore::new();
    let a1 = create(&store, new_run_in("deploy", &[("a", 1)])).await;
    assert_eq!(pick_id(&store).await, Some(a1.id));

    let a2 = create(&store, new_run_in("deploy", &[("a", 1)])).await;
    let b1 = create(&store, new_run_in("deploy", &[("b", 1)])).await;
    let free = create(&store, new_run("deploy")).await;

    assert_eq!(pick_id(&store).await, Some(b1.id));
    assert_eq!(pick_id(&store).await, Some(free.id));
    assert_eq!(pick_id(&store).await, None);

    let held = store.get_run(a2.id).await.unwrap().unwrap();
    assert_eq!(held.status.state, RunStatus::Pending);
}

#[tokio::test]
async fn pick_next_pending_uses_each_run_own_limit() {
    let store = InMemoryStore::new();
    let strict = create(&store, new_run_in("deploy", &[("g", 1)])).await;
    assert_eq!(pick_id(&store).await, Some(strict.id));

    // The strict run would be held back, the lenient one is still under its
    // own limit of 2.
    let blocked = create(&store, new_run_in("deploy", &[("g", 1)])).await;
    let lenient = create(&store, new_run_in("deploy", &[("g", 2)])).await;

    assert_eq!(pick_id(&store).await, Some(lenient.id));
    assert_eq!(pick_id(&store).await, None);
    let held = store.get_run(blocked.id).await.unwrap().unwrap();
    assert_eq!(held.status.state, RunStatus::Pending);
}

#[tokio::test]
async fn pick_next_pending_requires_every_group_under_limit() {
    let store = InMemoryStore::new();
    let a = create(&store, new_run_in("deploy", &[("a", 1)])).await;
    assert_eq!(pick_id(&store).await, Some(a.id));

    let multi = create(&store, new_run_in("deploy", &[("a", 1), ("b", 5)])).await;
    assert_eq!(pick_id(&store).await, None, "group a is saturated");

    store
        .update_run_status(a.id, RunStatus::Completed)
        .await
        .unwrap();
    assert_eq!(pick_id(&store).await, Some(multi.id));
}

#[tokio::test]
async fn pick_next_pending_does_not_count_sub_workflow_runs() {
    let store = InMemoryStore::new();
    let parent = create(&store, new_run_in("parent", &[("g", 2)])).await;
    assert_eq!(pick_id(&store).await, Some(parent.id));

    let child = create(
        &store,
        NewRun {
            trigger: TriggerKind::Workflow,
            ..new_run("child")
        },
    )
    .await;
    assert_eq!(pick_id(&store).await, Some(child.id));

    // Only the parent counts in g: one running out of two.
    let second = create(&store, new_run_in("parent", &[("g", 2)])).await;
    assert_eq!(pick_id(&store).await, Some(second.id));

    let third = create(&store, new_run_in("parent", &[("g", 2)])).await;
    assert_eq!(pick_id(&store).await, None);
    let held = store.get_run(third.id).await.unwrap().unwrap();
    assert_eq!(held.status.state, RunStatus::Pending);
}

#[tokio::test]
async fn pick_next_pending_sleeping_run_frees_group_slot() {
    let store = InMemoryStore::new();
    let a = create(&store, new_run_in("deploy", &[("g", 1)])).await;
    let b = create(&store, new_run_in("deploy", &[("g", 1)])).await;
    assert_eq!(pick_id(&store).await, Some(a.id));
    assert_eq!(pick_id(&store).await, None);

    store
        .update_run_status(a.id, RunStatus::Sleeping)
        .await
        .unwrap();

    assert_eq!(pick_id(&store).await, Some(b.id));
}

#[tokio::test]
async fn concurrent_picks_on_single_slot_group_only_one_wins() {
    let store = InMemoryStore::new();
    for _ in 0..10 {
        create(&store, new_run_in("deploy", &[("g", 1)])).await;
    }

    let mut handles = Vec::new();
    for _ in 0..20 {
        let s = store.clone();
        handles.push(tokio::spawn(async move { s.pick_next_pending(None).await }));
    }

    let mut picked = 0;
    for handle in handles {
        if handle.await.unwrap().unwrap().is_some() {
            picked += 1;
        }
    }
    assert_eq!(picked, 1, "a single-slot group admits exactly one run");

    let running = store
        .list_runs(
            RunFilter {
                status: Some(RunStatus::Running),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();
    assert_eq!(running.total, 1);
}

#[tokio::test]
async fn create_run_rejects_invalid_concurrency_limits() {
    let store = InMemoryStore::new();

    let cases: [&[(&str, u32)]; 3] = [&[("g", 0)], &[("", 1)], &[("g", 1), ("g", 2)]];
    for limits in cases {
        let err = store
            .create_run(new_run_in("deploy", limits))
            .await
            .unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidConcurrencyLimit(_)),
            "unexpected error for {limits:?}: {err:?}"
        );
    }

    let zero = store
        .create_run(new_run_in("deploy", &[("g", 0)]))
        .await
        .unwrap_err();
    assert!(matches!(
        zero,
        StoreError::InvalidConcurrencyLimit(ConcurrencyLimitError::ZeroLimit { .. })
    ));

    let page = store.list_runs(RunFilter::default(), 1, 100).await.unwrap();
    assert_eq!(page.total, 0, "an invalid run must not be stored");
}

#[tokio::test]
async fn create_run_stores_concurrency_limits() {
    let store = InMemoryStore::new();
    let limits = [("repo:acme", 2), ("tenant:42", 5)];
    let run = create(&store, new_run_in("deploy", &limits)).await;

    let fetched = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(
        fetched.concurrency_limits,
        vec![
            ConcurrencyLimit::new("repo:acme", 2),
            ConcurrencyLimit::new("tenant:42", 5),
        ]
    );
}

#[tokio::test]
async fn list_runs_filters_by_concurrency_group() {
    let store = InMemoryStore::new();
    let in_g = create(&store, new_run_in("deploy", &[("g", 1), ("h", 3)])).await;
    create(&store, new_run_in("deploy", &[("other", 1)])).await;
    create(&store, new_run("deploy")).await;

    let page = store
        .list_runs(
            RunFilter {
                concurrency_group: Some("g".to_string()),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].id, in_g.id);

    let none = store
        .list_runs(
            RunFilter {
                concurrency_group: Some("missing".to_string()),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();
    assert_eq!(none.total, 0);
}

#[tokio::test]
async fn count_blocked_runs_by_group_reports_saturated_groups() {
    let store = InMemoryStore::new();
    assert!(
        store
            .count_blocked_runs_by_group()
            .await
            .unwrap()
            .is_empty()
    );

    let a = create(&store, new_run_in("deploy", &[("a", 1)])).await;
    assert_eq!(pick_id(&store).await, Some(a.id));

    create(&store, new_run_in("deploy", &[("a", 1)])).await;
    create(&store, new_run_in("deploy", &[("a", 1), ("b", 1)])).await;
    // Still under its own limit: not counted.
    create(&store, new_run_in("deploy", &[("a", 3)])).await;
    // Not due yet: not counted.
    create(
        &store,
        NewRun {
            scheduled_at: Some(Utc::now() + TimeDelta::hours(1)),
            ..new_run_in("deploy", &[("a", 1)])
        },
    )
    .await;

    let backlog = store.count_blocked_runs_by_group().await.unwrap();
    assert_eq!(
        backlog,
        vec![ConcurrencyGroupBacklog {
            group: "a".to_string(),
            blocked_runs: 2,
        }]
    );
}

#[tokio::test]
async fn update_step_records_environment_id_and_keeps_it_on_later_updates() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap().into_run();
    let step = store
        .create_step(new_step(run.id, "agent", 0))
        .await
        .unwrap();
    assert_eq!(step.environment_id, None);

    store
        .update_step(
            step.id,
            StepUpdate {
                environment_id: Some("ironflow-env-0a1b2c".to_string()),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();
    store
        .update_step(
            step.id,
            StepUpdate {
                duration_ms: Some(10),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        store
            .get_step(step.id)
            .await
            .unwrap()
            .unwrap()
            .environment_id
            .as_deref(),
        Some("ironflow-env-0a1b2c")
    );
}

// ─── Capacity wait ──────────────────────────────────────────────

/// Create a run sleeping until `wake_at` on `kind` capacity, when given.
async fn capacity_sleeper(
    store: &InMemoryStore,
    wake_at: DateTime<Utc>,
    kind: Option<&str>,
) -> Uuid {
    let run = store
        .create_run(new_run("capacity"))
        .await
        .unwrap()
        .into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run(
            run.id,
            RunUpdate {
                status: Some(RunStatus::Sleeping),
                scheduled_at: Some(wake_at),
                capacity_wait_kind: kind.map(ProviderKind::from),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();
    run.id
}

#[tokio::test]
async fn capacity_wait_kind_is_kept_while_sleeping_and_cleared_after() {
    let store = InMemoryStore::new();
    let kind = "claude_subscription";
    let wake_at = Utc::now() + TimeDelta::hours(1);
    let run_id = capacity_sleeper(&store, wake_at, Some(kind)).await;

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.capacity_wait_kind, Some(ProviderKind::from(kind)));

    store
        .update_run(
            run_id,
            RunUpdate {
                scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();
    let woken = store.claim_due_sleeping_runs(10).await.unwrap();
    assert_eq!(woken.iter().map(|r| r.id).collect::<Vec<_>>(), vec![run_id]);
    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Pending);
    assert_eq!(run.capacity_wait_kind, None);
}

#[tokio::test]
async fn re_enabling_an_account_wakes_the_capacity_sleepers_of_its_kind() {
    let store = InMemoryStore::new();
    let mut disabled = new_account("perso", 10);
    disabled.enabled = false;
    let account = store.create_provider_account(disabled).await.unwrap();

    let wake_at = Utc::now() + TimeDelta::hours(1);
    let waiting = capacity_sleeper(&store, wake_at, Some("claude_subscription")).await;
    let other = capacity_sleeper(&store, wake_at, Some("other_kind")).await;
    let delayed = capacity_sleeper(&store, wake_at, None).await;

    store
        .update_provider_account(
            account.id,
            ProviderAccountUpdate {
                priority: Some(1),
                ..ProviderAccountUpdate::default()
            },
        )
        .await
        .unwrap();
    let run = store.get_run(waiting).await.unwrap().unwrap();
    assert_eq!(run.scheduled_at, Some(wake_at));

    store
        .update_provider_account(
            account.id,
            ProviderAccountUpdate {
                enabled: Some(true),
                ..ProviderAccountUpdate::default()
            },
        )
        .await
        .unwrap();
    let run = store.get_run(waiting).await.unwrap().unwrap();
    assert!(run.scheduled_at.expect("scheduled") <= Utc::now());
    for untouched in [other, delayed] {
        let run = store.get_run(untouched).await.unwrap().unwrap();
        assert_eq!(run.scheduled_at, Some(wake_at));
    }
}
