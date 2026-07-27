//! Integration tests for InMemoryStore covering all RunStore operations.

use std::collections::HashMap;

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
    }
}

fn new_step(run_id: Uuid, name: &str, position: u32) -> NewStep {
    NewStep {
        run_id,
        name: name.to_string(),
        kind: StepKind::Shell,
        position,
        input: None,
    }
}

// ─── CRUD Operations ────────────────────────────────────────────

#[tokio::test]
async fn create_and_retrieve_run() {
    let store = InMemoryStore::new();
    let created = store.create_run(new_run("test-workflow")).await.unwrap();

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
    let r1 = store.create_run(new_run("wf1")).await.unwrap();
    let r2 = store.create_run(new_run("wf2")).await.unwrap();
    let r3 = store.create_run(new_run("wf3")).await.unwrap();

    assert_ne!(r1.id, r2.id);
    assert_ne!(r2.id, r3.id);
    assert_ne!(r1.id, r3.id);
}

// ─── Filtering & Pagination ─────────────────────────────────────

#[tokio::test]
async fn list_runs_filters_by_workflow_name() {
    let store = InMemoryStore::new();
    store.create_run(new_run("deploy")).await.unwrap();
    store.create_run(new_run("test")).await.unwrap();
    store.create_run(new_run("deploy")).await.unwrap();
    store.create_run(new_run("build")).await.unwrap();

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
    let r1 = store.create_run(new_run("wf")).await.unwrap();
    let r2 = store.create_run(new_run("wf")).await.unwrap();
    let _r3 = store.create_run(new_run("wf")).await.unwrap();

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
    use chrono::Utc;

    let store = InMemoryStore::new();
    let before_time = Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let _r1 = store.create_run(new_run("wf")).await.unwrap();
    let after_time = Utc::now();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let _r2 = store.create_run(new_run("wf")).await.unwrap();

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
        store.create_run(new_run(&format!("wf-{i}"))).await.unwrap();
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
    store.create_run(new_run("wf")).await.unwrap();

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
    let r1 = store.create_run(new_run("wf1")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let r2 = store.create_run(new_run("wf2")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let r3 = store.create_run(new_run("wf3")).await.unwrap();

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
    let run = store.create_run(new_run("wf")).await.unwrap();

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
    let run = store.create_run(new_run("wf")).await.unwrap();

    // Pending → Completed is invalid
    let result = store.update_run_status(run.id, RunStatus::Completed).await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(matches!(err, StoreError::InvalidTransition { .. }));
}

#[tokio::test]
async fn update_run_status_terminal_state_to_terminal_errors() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap();

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
    let run = store.create_run(new_run("wf")).await.unwrap();

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
    let run = store.create_run(new_run("wf")).await.unwrap();
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
    let result = store.pick_next_pending().await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn pick_next_pending_returns_oldest_pending() {
    let store = InMemoryStore::new();
    let r1 = store.create_run(new_run("wf1")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let _r2 = store.create_run(new_run("wf2")).await.unwrap();

    let picked = store.pick_next_pending().await.unwrap().unwrap();
    assert_eq!(picked.id, r1.id);
    assert_eq!(picked.status.state, RunStatus::Running);
}

#[tokio::test]
async fn pick_next_pending_transitions_to_running() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap();
    assert_eq!(run.status.state, RunStatus::Pending);

    let picked = store.pick_next_pending().await.unwrap().unwrap();
    assert_eq!(picked.status.state, RunStatus::Running);
    assert!(picked.started_at.is_some());

    // Verify in store as well
    let fetched = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(fetched.status.state, RunStatus::Running);
}

#[tokio::test]
async fn pick_next_pending_skips_non_pending_runs() {
    let store = InMemoryStore::new();
    let r1 = store.create_run(new_run("wf1")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let r2 = store.create_run(new_run("wf2")).await.unwrap();

    // Transition r1 to Running
    store
        .update_run_status(r1.id, RunStatus::Running)
        .await
        .unwrap();

    // Should pick r2 (the next oldest pending)
    let picked = store.pick_next_pending().await.unwrap().unwrap();
    assert_eq!(picked.id, r2.id);
}

// ─── Steps ──────────────────────────────────────────────────────

#[tokio::test]
async fn create_step_for_existing_run() {
    let store = InMemoryStore::new();
    let run = store.create_run(new_run("wf")).await.unwrap();

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
    let run = store.create_run(new_run("wf")).await.unwrap();

    // Insert out of order
    store
        .create_step(NewStep {
            run_id: run.id,
            name: "step3".to_string(),
            kind: StepKind::Shell,
            position: 2,
            input: None,
        })
        .await
        .unwrap();

    store
        .create_step(NewStep {
            run_id: run.id,
            name: "step1".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: None,
        })
        .await
        .unwrap();

    store
        .create_step(NewStep {
            run_id: run.id,
            name: "step2".to_string(),
            kind: StepKind::Shell,
            position: 1,
            input: None,
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
    let run = store.create_run(new_run("wf")).await.unwrap();

    let steps = store.list_steps(run.id).await.unwrap();
    assert!(steps.is_empty());
}

#[tokio::test]
async fn list_steps_filters_by_run_id() {
    let store = InMemoryStore::new();
    let run1 = store.create_run(new_run("wf1")).await.unwrap();
    let run2 = store.create_run(new_run("wf2")).await.unwrap();

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
    let run = store.create_run(new_run("wf")).await.unwrap();
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

    let r1 = store.create_run(new_run("wf")).await.unwrap();
    let r2 = store.create_run(new_run("wf")).await.unwrap();
    let r3 = store.create_run(new_run("wf")).await.unwrap();
    let _r4 = store.create_run(new_run("wf")).await.unwrap();

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

    let r1 = store.create_run(new_run("wf")).await.unwrap();
    let r2 = store.create_run(new_run("wf")).await.unwrap();

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

    let r1 = store.create_run(new_run("wf")).await.unwrap();
    let r2 = store.create_run(new_run("wf")).await.unwrap();

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

    let r1 = store.create_run(new_run("wf")).await.unwrap();
    let r2 = store.create_run(new_run("wf")).await.unwrap();
    let r3 = store.create_run(new_run("wf")).await.unwrap();
    let _r4 = store.create_run(new_run("wf")).await.unwrap(); // Pending

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
    let run = store.create_run(new_run("wf")).await.unwrap();

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
    let run = store.create_run(new_run("wf")).await.unwrap();

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
    let run = store.create_run(new_run("wf")).await.unwrap();
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
    store.create_run(new_run("wf")).await.unwrap();

    // Page 0 should be clamped to page 1
    let page = store.list_runs(RunFilter::default(), 0, 10).await.unwrap();
    assert_eq!(page.page, 1);
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn pagination_edge_case_per_page_zero_defaults_to_one() {
    let store = InMemoryStore::new();
    for i in 0..5 {
        store.create_run(new_run(&format!("wf-{i}"))).await.unwrap();
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
        store.create_run(new_run(&format!("wf-{i}"))).await.unwrap();
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
            s.create_run(new_run(&format!("wf-{i}"))).await
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
    };

    let run = store.create_run(req).await.unwrap();
    let retrieved = store.get_run(run.id).await.unwrap().unwrap();

    assert_eq!(retrieved.payload, large_payload);
}
