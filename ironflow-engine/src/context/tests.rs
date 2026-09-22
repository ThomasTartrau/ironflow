//! Unit tests for [`WorkflowContext`].
//!
//! A descendant module of `context`, so the tests can assert on the private
//! fields the public API only exposes indirectly.

use super::*;
use chrono::{TimeDelta, Utc};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    Assignee, NewRun, NewStep, Run, RunActor, RunFilter, StepKind, StepStatus, StepUpdate,
    TriggerKind, step_trace_id,
};
use ironflow_store::store::RunStore;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

use crate::config::ApprovalConfig;
use crate::error::EngineError;

/// Helper to create a test provider with fixtures
fn create_test_provider() -> Arc<dyn ironflow_core::provider::AgentProvider> {
    let inner = ClaudeCodeProvider::new();
    Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ))
}

/// Helper to create a test context
fn create_test_context() -> WorkflowContext {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();
    let run_id = Uuid::now_v7();
    WorkflowContext::new(run_id, "test".to_string(), store, provider)
}

#[test]
fn context_new_initializes_correctly() {
    let ctx = create_test_context();
    assert_eq!(ctx.position, 0);
    assert_eq!(ctx.total_cost_usd, Decimal::ZERO);
    assert_eq!(ctx.total_duration_ms, 0);
    assert!(ctx.last_step_ids.is_empty());
    assert!(ctx.replay_steps.is_empty());
    assert!(ctx.log_sender.is_none());
}

#[test]
fn context_run_id_returns_correct_id() {
    let run_id = Uuid::now_v7();
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();
    let ctx = WorkflowContext::new(run_id, "test".to_string(), store, provider);
    assert_eq!(ctx.run_id(), run_id);
}

#[test]
fn context_total_cost_usd_initially_zero() {
    let ctx = create_test_context();
    assert_eq!(ctx.total_cost_usd(), Decimal::ZERO);
}

#[test]
fn context_total_duration_ms_initially_zero() {
    let ctx = create_test_context();
    assert_eq!(ctx.total_duration_ms(), 0);
}

#[test]
fn context_with_handler_resolver_creates_context_with_resolver() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();
    let run_id = Uuid::now_v7();

    let called = Arc::new(AtomicBool::new(false));
    let called_clone = called.clone();

    let resolver: HandlerResolver = Arc::new(move |_name: &str| {
        called_clone.store(true, Ordering::SeqCst);
        None
    });

    let ctx = WorkflowContext::with_handler_resolver(
        run_id,
        "test".to_string(),
        store,
        provider,
        resolver,
    );

    assert_eq!(ctx.run_id(), run_id);
    assert!(ctx.handler_resolver.is_some());
}

#[tokio::test]
async fn context_set_log_sender_attaches_sender() {
    let mut ctx = create_test_context();
    let (sender, _receiver) = crate::log_sender::channel();
    ctx.set_log_sender(sender);
    assert!(ctx.log_sender.is_some());
}

#[tokio::test]
async fn context_skip_creates_skipped_step() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

    // Create the run first using RunStore trait
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create run")
        .into_run();

    // Get the created run to extract its ID
    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    let mut ctx = WorkflowContext::new(created_run_id, "test".to_string(), store.clone(), provider);
    let initial_position = ctx.position;

    ctx.skip("skip-step", "condition not met")
        .await
        .expect("skip failed");

    assert_eq!(ctx.position, initial_position + 1);
    assert!(!ctx.last_step_ids.is_empty());

    // Verify the step was recorded with Skipped status
    let steps = store
        .list_steps(created_run_id)
        .await
        .expect("failed to list steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status.state, StepStatus::Skipped);
}

/// Sub-workflow handler that records no steps, so the child run reaches a
/// terminal state without touching the filesystem or the network.
struct NoopSubWorkflow;

impl WorkflowHandler for NoopSubWorkflow {
    fn name(&self) -> &str {
        "noop-sub"
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> crate::handler::HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

/// Run a parent workflow authored by `created_by` and return the child run
/// created by its sub-workflow step.
async fn child_run_of_parent_authored_by(created_by: Option<RunActor>) -> Run {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

    let parent = store
        .create_run(NewRun {
            workflow_name: "parent".to_string(),
            trigger: TriggerKind::Api,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            created_by,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create parent run")
        .into_run();

    let resolver: HandlerResolver = Arc::new(|name: &str| match name {
        "noop-sub" => Some(Arc::new(NoopSubWorkflow) as Arc<dyn WorkflowHandler>),
        _ => None,
    });

    let mut ctx = WorkflowContext::with_handler_resolver(
        parent.id,
        "parent".to_string(),
        store.clone(),
        provider,
        resolver,
    );
    ctx.workflow(&NoopSubWorkflow, json!({}))
        .await
        .expect("sub-workflow failed");

    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    runs.items
        .into_iter()
        .find(|r| r.workflow_name == "noop-sub")
        .expect("child run was created")
}

#[tokio::test]
async fn child_run_inherits_the_parent_author() {
    let user_id = Uuid::now_v7();
    let child = child_run_of_parent_authored_by(Some(RunActor::User { user_id })).await;

    assert_eq!(child.created_by, Some(RunActor::User { user_id }));
}

#[tokio::test]
async fn child_run_of_an_unattributed_parent_has_no_author() {
    let child = child_run_of_parent_authored_by(None).await;

    assert!(child.created_by.is_none());
}

#[tokio::test]
async fn context_parallel_empty_steps_returns_empty_vec() {
    let mut ctx = create_test_context();
    let results = ctx
        .parallel(vec![], true)
        .await
        .expect("parallel should not fail on empty input");
    assert!(results.is_empty());
}

#[tokio::test]
async fn context_approval_first_execution_returns_error() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

    // Create the run first
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create run")
        .into_run();

    // Get the created run to extract its ID
    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    let mut ctx = WorkflowContext::new(created_run_id, "test".to_string(), store.clone(), provider);

    let result = ctx
        .approval(
            "approve-step",
            crate::config::ApprovalConfig::new("Continue?"),
        )
        .await;

    // First execution should return ApprovalRequired error
    assert!(matches!(result, Err(EngineError::ApprovalRequired { .. })));

    // Verify position incremented
    assert_eq!(ctx.position, 1);

    // Verify step was created with AwaitingApproval status
    let steps = store
        .list_steps(created_run_id)
        .await
        .expect("failed to list steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status.state, StepStatus::AwaitingApproval);
}

#[tokio::test]
async fn context_approval_replay_returns_ok() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

    // Create the run first
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create run")
        .into_run();

    // Get the created run to extract its ID
    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    // Create an approval step that's already in AwaitingApproval state
    let step = store
        .create_step(NewStep {
            run_id: created_run_id,
            trace_id: step_trace_id(created_run_id, "approval", 0),
            name: "approval".to_string(),
            kind: StepKind::Approval,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("failed to create step");

    // Transition through proper states: Pending -> Running -> AwaitingApproval
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                started_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to update step to Running");

    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::AwaitingApproval),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to update step to AwaitingApproval");

    // Create context and load replay steps
    let mut ctx = WorkflowContext::new(created_run_id, "test".to_string(), store.clone(), provider);
    ctx.load_replay_steps()
        .await
        .expect("failed to load replay steps");

    // Now approval should succeed (replay)
    let result = ctx
        .approval("approval", crate::config::ApprovalConfig::new("Continue?"))
        .await;

    assert!(result.is_ok());

    // Verify the step was marked Completed
    let steps = store
        .list_steps(created_run_id)
        .await
        .expect("failed to list steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status.state, StepStatus::Completed);
}

#[tokio::test]
async fn context_load_replay_steps_loads_completed_steps() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

    // Create the run first
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create run")
        .into_run();

    // Get the created run to extract its ID
    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    // Create multiple steps with different statuses
    let completed_step = store
        .create_step(NewStep {
            run_id: created_run_id,
            trace_id: step_trace_id(created_run_id, "completed", 0),
            name: "completed".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("failed to create step");

    // Transition to Running then Completed
    store
        .update_step(
            completed_step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                started_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to update step to Running");

    store
        .update_step(
            completed_step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                completed_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to update step to Completed");

    let _pending_step = store
        .create_step(NewStep {
            run_id: created_run_id,
            trace_id: step_trace_id(created_run_id, "pending", 1),
            name: "pending".to_string(),
            kind: StepKind::Shell,
            position: 1,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("failed to create step");

    // Load replay steps
    let mut ctx = WorkflowContext::new(created_run_id, "test".to_string(), store, provider);
    ctx.load_replay_steps()
        .await
        .expect("failed to load replay steps");

    // Only completed step should be in replay_steps
    assert_eq!(ctx.replay_steps.len(), 1);
    assert!(ctx.replay_steps.contains_key(&0));
    assert!(!ctx.replay_steps.contains_key(&1));
}

#[tokio::test]
async fn context_payload_returns_run_payload() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();
    let test_payload = json!({"key": "value", "number": 42});

    // Create the run first
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: test_payload.clone(),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create run")
        .into_run();

    // Get the created run to extract its ID
    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    let ctx = WorkflowContext::new(created_run_id, "test".to_string(), store, provider);
    let payload = ctx.payload().await.expect("failed to get payload");

    assert_eq!(payload, test_payload);
}

#[tokio::test]
async fn context_payload_returns_error_for_nonexistent_run() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();
    let run_id = Uuid::now_v7();

    let ctx = WorkflowContext::new(run_id, "test".to_string(), store, provider);
    let result = ctx.payload().await;

    assert!(result.is_err());
}

#[tokio::test]
async fn context_store_returns_reference() {
    let ctx = create_test_context();
    let _store = ctx.store();
    // store() returns a reference to the Arc<dyn Store>, which is always available
}

#[test]
fn context_debug_formatting() {
    let ctx = create_test_context();
    let debug_str = format!("{:?}", ctx);
    assert!(debug_str.contains("WorkflowContext"));
    assert!(debug_str.contains("run_id"));
}

#[tokio::test]
async fn context_last_step_ids_tracks_executed_steps() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

    // Create the run first
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create run")
        .into_run();

    // Get the created run to extract its ID
    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    let mut ctx = WorkflowContext::new(created_run_id, "test".to_string(), store, provider);
    assert!(ctx.last_step_ids.is_empty());

    ctx.skip("step1", "reason").await.expect("skip failed");

    assert_eq!(ctx.last_step_ids.len(), 1);

    ctx.skip("step2", "reason").await.expect("skip failed");

    // last_step_ids should now contain only step2's ID
    assert_eq!(ctx.last_step_ids.len(), 1);
}

// -- approval SLA timers --

/// A context wired to a freshly created run on a shared store.
async fn context_with_run() -> (Arc<InMemoryStore>, WorkflowContext) {
    let store = Arc::new(InMemoryStore::new());
    let run = store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create run")
        .into_run();

    let ctx = WorkflowContext::new(
        run.id,
        "test".to_string(),
        store.clone(),
        create_test_provider(),
    );
    (store, ctx)
}

#[tokio::test]
async fn approval_without_deadline_leaves_timer_unset() {
    let (store, mut ctx) = context_with_run().await;

    let err = ctx
        .approval("gate", ApprovalConfig::new("Approve?"))
        .await
        .expect_err("approval suspends the run");
    assert!(matches!(err, EngineError::ApprovalRequired { .. }));

    let steps = store.list_steps(ctx.run_id()).await.expect("list steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status.state, StepStatus::AwaitingApproval);
    assert!(steps[0].approval_deadline_at.is_none());
    assert_eq!(steps[0].approval_stage, 0);
    assert!(steps[0].approval_assignee.is_none());
}

#[tokio::test]
async fn approval_with_deadline_arms_timer() {
    let (store, mut ctx) = context_with_run().await;

    let before = Utc::now();
    let config = ApprovalConfig::new("Approve?")
        .with_deadline_secs(3600)
        .assigned_to(Assignee::group("release-managers"));
    ctx.approval("gate", config)
        .await
        .expect_err("approval suspends the run");

    let steps = store.list_steps(ctx.run_id()).await.expect("list steps");
    let deadline = steps[0].approval_deadline_at.expect("timer is armed");
    assert!(deadline >= before + TimeDelta::seconds(3600));
    assert!(deadline <= Utc::now() + TimeDelta::seconds(3600));
    assert_eq!(steps[0].approval_stage, 0);
    assert_eq!(
        steps[0].approval_assignee,
        Some(Assignee::group("release-managers"))
    );
}

#[tokio::test]
async fn approval_honours_the_legacy_timeout_seconds() {
    let (store, mut ctx) = context_with_run().await;

    ctx.approval(
        "gate",
        ApprovalConfig::new("Approve?").with_timeout_seconds(60),
    )
    .await
    .expect_err("approval suspends the run");

    let steps = store.list_steps(ctx.run_id()).await.expect("list steps");
    assert!(steps[0].approval_deadline_at.is_some());
}

#[tokio::test]
async fn approval_replay_clears_deadline() {
    let (store, mut ctx) = context_with_run().await;

    ctx.approval(
        "gate",
        ApprovalConfig::new("Approve?").with_deadline_secs(3600),
    )
    .await
    .expect_err("approval suspends the run");

    // Replay the handler the way resume_run does.
    let mut resumed = WorkflowContext::new(
        ctx.run_id(),
        "test".to_string(),
        store.clone(),
        create_test_provider(),
    );
    resumed
        .load_replay_steps()
        .await
        .expect("load replay steps");
    resumed
        .approval(
            "gate",
            ApprovalConfig::new("Approve?").with_deadline_secs(3600),
        )
        .await
        .expect("replayed gate continues");

    let steps = store.list_steps(ctx.run_id()).await.expect("list steps");
    assert_eq!(steps[0].status.state, StepStatus::Completed);
    assert!(steps[0].approval_deadline_at.is_none());
}

#[tokio::test]
async fn when_applies_the_predicate_to_the_payload() {
    let store = Arc::new(InMemoryStore::new());
    let run = store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({"env": "prod"}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("failed to create run")
        .into_run();

    let mut ctx = WorkflowContext::new(
        run.id,
        "test".to_string(),
        store.clone(),
        create_test_provider(),
    );

    assert!(
        ctx.when("env == prod", |p| p["env"] == "prod")
            .await
            .expect("condition evaluated")
    );
    assert!(
        !ctx.when("env == dev", |p| p["env"] == "dev")
            .await
            .expect("condition evaluated")
    );
}

#[test]
fn when_dynamic_returns_its_argument_unchanged() {
    let mut ctx = create_test_context();
    assert!(ctx.when_dynamic("build succeeded", true));
    assert!(!ctx.when_dynamic("build succeeded", false));
}

#[test]
fn a_normal_context_is_not_planning() {
    let ctx = create_test_context();
    assert!(!ctx.is_planning());
    assert!(ctx.plan().is_none());
}
