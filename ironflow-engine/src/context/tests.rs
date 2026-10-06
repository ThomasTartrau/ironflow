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
    Assignee, NewRun, NewStep, Run, RunActor, RunFilter, RunStatus, StepKind, StepStatus,
    StepUpdate, TriggerKind, step_trace_id,
};
use ironflow_store::store::RunStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

use crate::config::delay::DelayConfig;
use crate::config::{ApprovalConfig, DecisionConfig, HumanInputConfig, ShellConfig, StepConfig};
use crate::decision::DecisionAnswers;
use crate::error::EngineError;
use crate::handler::TypedWorkflow;
use crate::plan::PlanRecorder;
use crate::testing::{MockInterceptor, MockShellOutput};

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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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

impl TypedWorkflow for NoopSubWorkflow {
    type Input = ();
}

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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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
    ctx.workflow(&NoopSubWorkflow, ())
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

struct GpuSubWorkflow;

impl TypedWorkflow for GpuSubWorkflow {
    type Input = ();
}

impl WorkflowHandler for GpuSubWorkflow {
    fn name(&self) -> &str {
        "gpu-sub"
    }

    fn required_worker_tags(&self) -> Vec<String> {
        vec!["gpu".to_string()]
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> crate::handler::HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

/// Run a parent whose context carries `worker_tags` and calls
/// [`GpuSubWorkflow`]. Returns the step outcome and the child run, if any.
async fn run_gpu_child(worker_tags: Option<Vec<String>>) -> (Result<(), EngineError>, Option<Run>) {
    let store = Arc::new(InMemoryStore::new());
    let parent = store
        .create_run(NewRun {
            workflow_name: "parent".to_string(),
            trigger: TriggerKind::Api,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: Default::default(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("failed to create parent run")
        .into_run();

    let resolver: HandlerResolver = Arc::new(|name: &str| match name {
        "gpu-sub" => Some(Arc::new(GpuSubWorkflow) as Arc<dyn WorkflowHandler>),
        _ => None,
    });
    let mut ctx = WorkflowContext::with_handler_resolver(
        parent.id,
        "parent".to_string(),
        store.clone(),
        create_test_provider(),
        resolver,
    );
    if let Some(tags) = worker_tags {
        ctx.set_worker_tags(Arc::new(tags));
    }

    let result = ctx.workflow(&GpuSubWorkflow, ()).await.map(|_| ());
    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let child = runs
        .items
        .into_iter()
        .find(|r| r.workflow_name == "gpu-sub");
    (result, child)
}

#[tokio::test]
async fn sub_workflow_requiring_missing_worker_tag_fails_with_clear_error() {
    let (result, child) = run_gpu_child(Some(vec!["arm".to_string()])).await;

    let Err(EngineError::InvalidWorkflow(message)) = result else {
        panic!("expected an invalid workflow error, got {result:?}");
    };
    assert_eq!(
        message,
        "sub-workflow 'gpu-sub' requires worker tags [gpu] that this worker does not carry"
    );
    assert!(child.is_none(), "no child run is created");
}

#[tokio::test]
async fn sub_workflow_whose_worker_tags_are_carried_runs() {
    let (result, child) = run_gpu_child(Some(vec!["gpu".to_string(), "arm".to_string()])).await;

    assert!(result.is_ok(), "{result:?}");
    let child = child.expect("child run was created");
    assert_eq!(child.worker_tags, vec!["gpu".to_string()]);
    assert_eq!(child.status.state, RunStatus::Completed);
}

#[tokio::test]
async fn sub_workflow_worker_tags_are_not_checked_outside_a_worker() {
    let (result, child) = run_gpu_child(None).await;

    assert!(result.is_ok(), "{result:?}");
    assert!(child.is_some());
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
async fn context_parallel_rejects_duplicate_step_names_before_running() {
    let mut ctx = create_test_context();
    let err = ctx
        .parallel(
            vec![
                ("fetch", StepConfig::Shell(ShellConfig::new("echo alpha"))),
                ("other", StepConfig::Shell(ShellConfig::new("echo other"))),
                ("fetch", StepConfig::Shell(ShellConfig::new("echo beta"))),
            ],
            true,
        )
        .await
        .expect_err("a wave with two steps named 'fetch' must be refused");

    assert!(
        matches!(&err, EngineError::StepConfig(msg) if msg.contains("\"fetch\"")),
        "unexpected error: {err}"
    );
    assert_eq!(ctx.position, 0, "a refused wave takes no position");
    let steps = ctx.store.list_steps(ctx.run_id).await.expect("list steps");
    assert!(
        steps.is_empty(),
        "no step of a refused wave may be recorded"
    );
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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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
async fn context_load_replay_steps_keeps_the_oldest_completed_step_on_position_collision() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("failed to create run")
        .into_run();

    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    // Two steps recorded at the same position (pre-fix duplication bug):
    // the older one is the one that actually produced the side effects.
    let older = store
        .create_step(NewStep {
            run_id: created_run_id,
            trace_id: step_trace_id(created_run_id, "dup", 0),
            name: "dup".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("failed to create step");
    store
        .update_step(
            older.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                started_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to start older step");
    store
        .update_step(
            older.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                completed_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to complete older step");

    tokio::time::sleep(Duration::from_millis(5)).await;

    let newer = store
        .create_step(NewStep {
            run_id: created_run_id,
            trace_id: step_trace_id(created_run_id, "dup", 0),
            name: "dup".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("failed to create step");
    store
        .update_step(
            newer.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                started_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to start newer step");
    store
        .update_step(
            newer.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                completed_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to complete newer step");

    let mut ctx = WorkflowContext::new(created_run_id, "test".to_string(), store, provider);
    ctx.load_replay_steps()
        .await
        .expect("failed to load replay steps");

    let replayed = ctx.replay_steps.get(&0).expect("a replay candidate");
    assert_eq!(replayed.id, older.id);
}

#[tokio::test]
async fn context_load_replay_steps_populates_replay_wave_steps() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("failed to create run")
        .into_run();

    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    // A parallel wave: two steps sharing one position, distinct names.
    for name in ["wave-a", "wave-b"] {
        let step = store
            .create_step(NewStep {
                run_id: created_run_id,
                trace_id: step_trace_id(created_run_id, name, 0),
                name: name.to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("failed to create step");
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
            .expect("failed to start step");
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    completed_at: Some(Utc::now()),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("failed to complete step");
    }

    let mut ctx = WorkflowContext::new(created_run_id, "test".to_string(), store, provider);
    ctx.load_replay_steps()
        .await
        .expect("failed to load replay steps");

    assert_eq!(ctx.replay_wave_steps.len(), 2);
    assert!(
        ctx.replay_wave_steps
            .contains_key(&(0, "wave-a".to_string()))
    );
    assert!(
        ctx.replay_wave_steps
            .contains_key(&(0, "wave-b".to_string()))
    );
}

#[tokio::test]
async fn context_load_replay_steps_includes_skipped_steps() {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("failed to create run")
        .into_run();

    let runs = store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("failed to list runs");
    let created_run_id = runs.items[0].id;

    let step = store
        .create_step(NewStep {
            run_id: created_run_id,
            trace_id: step_trace_id(created_run_id, "maybe", 0),
            name: "maybe".to_string(),
            kind: StepKind::Custom("skip".to_string()),
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("failed to create step");
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Skipped),
                output: Some(json!({"reason": "not needed"})),
                completed_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to mark step skipped");

    let mut ctx = WorkflowContext::new(created_run_id, "test".to_string(), store, provider);
    ctx.load_replay_steps()
        .await
        .expect("failed to load replay steps");

    let replayed = ctx.replay_steps.get(&0).expect("skipped step is replayed");
    assert_eq!(replayed.id, step.id);
    assert_eq!(replayed.status.state, StepStatus::Skipped);
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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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

// -- step interceptor --

#[tokio::test]
async fn set_step_interceptor_short_circuits_a_shell_step() {
    let (store, mut ctx) = context_with_run().await;
    // `exit 1` would fail the step if a process were really spawned.
    let mocks = MockInterceptor::new().shell(|_cfg| Ok(MockShellOutput::ok("mocked")));
    ctx.set_step_interceptor(Arc::new(mocks));

    let output = ctx
        .shell("build", ShellConfig::new("exit 1"))
        .await
        .expect("the interceptor resolved the step");

    assert_eq!(output.stdout(), "mocked");

    let steps = store.list_steps(ctx.run_id()).await.expect("list steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status.state, StepStatus::Completed);
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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
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
        ctx.when("production run", |i: &EnvInput| i.env == "prod")
            .await
            .expect("condition evaluated")
    );
    assert!(
        !ctx.when("development run", |i: &EnvInput| i.env == "dev")
            .await
            .expect("condition evaluated")
    );
}

#[derive(Deserialize)]
struct EnvInput {
    env: String,
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

/// The answer type used by [`context_decision_replay_divergence_on_kind_mismatch`].
///
/// The divergence error is returned before any answer is ever read, so the
/// field only needs to exist for `DecisionConfig::answers::<T>()` to compile.
#[allow(dead_code)]
#[derive(Debug, DecisionAnswers)]
struct DivergenceAnswers {
    #[noul("Does this convey urgency?")]
    is_urgent: f64,
}

/// Create a run with a single `Completed` step at position 0 under `name`/`kind`,
/// then a context with that step already loaded into `replay_steps`.
///
/// Mirrors `context_approval_replay_returns_ok`'s Pending -> Running ->
/// (terminal) transition sequence, which the in-memory store's step FSM
/// requires.
async fn context_with_replayed_step_at_position_zero(
    name: &str,
    kind: StepKind,
) -> WorkflowContext {
    let store = Arc::new(InMemoryStore::new());
    let provider = create_test_provider();

    let run_id = store
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
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("failed to create run")
        .into_run()
        .id;

    let step = store
        .create_step(NewStep {
            run_id,
            trace_id: step_trace_id(run_id, name, 0),
            name: name.to_string(),
            kind,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("failed to create step");

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
                status: Some(StepStatus::Completed),
                output: Some(json!({})),
                completed_at: Some(Utc::now()),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("failed to update step to Completed");

    let mut ctx = WorkflowContext::new(run_id, "test".to_string(), store.clone(), provider);
    ctx.load_replay_steps()
        .await
        .expect("failed to load replay steps");
    ctx
}

#[tokio::test]
async fn context_approval_replay_divergence_on_name_mismatch() {
    let mut ctx = context_with_replayed_step_at_position_zero("old", StepKind::Approval).await;

    let result = ctx.approval("new", ApprovalConfig::new("Continue?")).await;

    assert!(matches!(
        result.unwrap_err(),
        EngineError::ReplayDivergence { position: 0, .. }
    ));
}

/// The answer type used by [`context_human_input_replay_divergence_on_name_mismatch`].
#[derive(Debug, Deserialize, JsonSchema)]
struct DivergenceHumanInputAnswers {
    #[allow(dead_code)]
    answers: Vec<String>,
}

#[tokio::test]
async fn context_human_input_replay_divergence_on_name_mismatch() {
    let mut ctx = context_with_replayed_step_at_position_zero("old", StepKind::HumanInput).await;

    let result = ctx
        .human_input::<DivergenceHumanInputAnswers>("new", HumanInputConfig::new("Answer?"))
        .await;

    assert!(matches!(
        result.unwrap_err(),
        EngineError::ReplayDivergence { position: 0, .. }
    ));
}

#[tokio::test]
async fn context_decision_replay_divergence_on_kind_mismatch() {
    let mut ctx = context_with_replayed_step_at_position_zero("triage", StepKind::Shell).await;

    let result = ctx
        .decision(
            "triage",
            DecisionConfig::new("state").answers::<DivergenceAnswers>(),
        )
        .await;

    assert!(matches!(
        result.unwrap_err(),
        EngineError::ReplayDivergence { position: 0, .. }
    ));
}

#[tokio::test]
async fn context_delay_replay_divergence_on_name_mismatch() {
    let mut ctx =
        context_with_replayed_step_at_position_zero("old", StepKind::Custom("delay".to_string()))
            .await;

    let result = ctx.delay("new", DelayConfig::from_secs(60)).await;

    assert!(matches!(
        result.unwrap_err(),
        EngineError::ReplayDivergence { position: 0, .. }
    ));
}

// -- set_output --

/// A map with non-string keys: `serde_json` refuses to serialize it.
fn unserializable() -> HashMap<(u8, u8), u8> {
    HashMap::from([((1, 2), 3)])
}

#[test]
fn set_output_is_none_on_a_new_context() {
    let ctx = create_test_context();
    assert!(ctx.output().is_none());
}

#[test]
fn set_output_stores_the_serialized_value() {
    let mut ctx = create_test_context();
    ctx.set_output(&json!({"approved": true}))
        .expect("serializable");
    assert_eq!(ctx.output(), Some(&json!({"approved": true})));
}

#[test]
fn set_output_last_call_wins_in_the_context() {
    let mut ctx = create_test_context();
    ctx.set_output(&"first").expect("serializable");
    ctx.set_output(&"second").expect("serializable");
    assert_eq!(ctx.output(), Some(&json!("second")));
}

#[test]
fn set_output_rejects_an_unserializable_value() {
    let mut ctx = create_test_context();
    ctx.set_output(&"kept").expect("serializable");

    let err = ctx
        .set_output(&unserializable())
        .expect_err("non-string map keys do not serialize");

    assert!(matches!(err, EngineError::Serialization(_)), "got {err:?}");
    assert_eq!(
        ctx.output(),
        Some(&json!("kept")),
        "a failed call leaves the previous output in place"
    );
}

#[test]
fn set_output_is_a_noop_on_a_planning_context() {
    let mut ctx = create_test_context();
    ctx.set_plan(Arc::new(Mutex::new(PlanRecorder::new(
        "test".to_string(),
        json!({}),
        3,
        HashMap::new(),
    ))));

    ctx.set_output(&unserializable())
        .expect("nothing is serialized while planning");
    ctx.set_output(&"ignored").expect("no-op");
    assert!(ctx.output().is_none());
}
