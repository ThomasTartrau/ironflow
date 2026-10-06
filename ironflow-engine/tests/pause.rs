//! Operator pause of a run and of a workflow.
//!
//! Every test drives a real [`Engine`] over a real [`InMemoryStore`], under
//! [`ExecutionMode::Workers`]: runs are picked from the store the way a
//! worker does, so a resumed run waits in the queue instead of restarting in
//! the background. Test names start with `pause_` so `cargo test -p
//! ironflow-engine pause_` selects them.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::spawn;
use tokio::sync::Notify;
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::error::OperationError;
use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::ApprovalConfig;
use ironflow_engine::config::delay::DelayConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::operation::{Operation, OperationContext};
use ironflow_store::error::StoreError;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, RunUpdate, StepStatus, TriggerKind};
use ironflow_store::store::{RunStore, STEP_INTERRUPTED_ERROR, Store};

/// Test timeout for bodies that touch the store and spawn tasks.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name of every top-level handler in this file.
const WORKFLOW: &str = "pause";

/// Workflow name of the sub-workflows in this file.
const CHILD_WORKFLOW: &str = "pause-child";

/// A custom operation that counts how many times it actually ran.
#[derive(Clone)]
struct CountingOp {
    calls: Arc<AtomicU32>,
}

impl CountingOp {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicU32::new(0)),
        }
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Operation for CountingOp {
    fn kind(&self) -> &str {
        "counting-op"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"called": true}))
    }
}

/// A custom operation that signals it started, then waits to be released:
/// the test pauses the run while this step is in flight.
#[derive(Clone)]
struct GateOp {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    calls: Arc<AtomicU32>,
}

impl GateOp {
    fn new() -> Self {
        Self {
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            calls: Arc::new(AtomicU32::new(0)),
        }
    }
}

#[async_trait]
impl Operation for GateOp {
    fn kind(&self) -> &str {
        "gate-op"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(json!({"released": true}))
    }
}

/// A gated step, then a counting step.
struct GateThenCount {
    gate: GateOp,
    after: CountingOp,
}

impl WorkflowHandler for GateThenCount {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("gate", &self.gate).await?;
            ctx.operation("after", &self.after).await?;
            Ok(())
        })
    }
}

/// A single counting step.
struct Single {
    op: CountingOp,
}

impl WorkflowHandler for Single {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("only", &self.op).await?;
            Ok(())
        })
    }
}

/// A five-minute delay.
struct Sleeper;

impl WorkflowHandler for Sleeper {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.delay("wait-5min", DelayConfig::from_secs(300)).await?;
            Ok(())
        })
    }
}

/// Input of the child workflows.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ChildInput {}

/// A child that waits for an approval.
#[derive(Clone)]
struct ApprovalChild;

impl WorkflowHandler for ApprovalChild {
    fn name(&self) -> &str {
        CHILD_WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.approval("child-gate", ApprovalConfig::new("Go on?"))
                .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for ApprovalChild {
    type Input = ChildInput;
}

/// A parent whose only step is [`ApprovalChild`].
struct ApprovalParent;

impl WorkflowHandler for ApprovalParent {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&ApprovalChild, ChildInput {}).await?;
            Ok(())
        })
    }
}

/// A child with a gated step, then a counting step.
#[derive(Clone)]
struct GatedChild {
    gate: GateOp,
    after: CountingOp,
}

impl WorkflowHandler for GatedChild {
    fn name(&self) -> &str {
        CHILD_WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("child-gate", &self.gate).await?;
            ctx.operation("child-after", &self.after).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for GatedChild {
    type Input = ChildInput;
}

/// [`GatedChild`], then a counting step.
struct GatedParent {
    child: GatedChild,
    after: CountingOp,
}

impl WorkflowHandler for GatedParent {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&self.child, ChildInput {}).await?;
            ctx.operation("parent-after", &self.after).await?;
            Ok(())
        })
    }
}

fn new_engine(store: &Arc<InMemoryStore>) -> Engine {
    let store: Arc<dyn Store> = store.clone();
    let provider: Arc<dyn AgentProvider> = Arc::new(ClaudeCodeProvider::new());
    Engine::new(store, provider).with_execution_mode(ExecutionMode::Workers)
}

fn engine_with(store: &Arc<InMemoryStore>, handler: impl WorkflowHandler + 'static) -> Arc<Engine> {
    let mut engine = new_engine(store);
    engine.register(handler).expect("register handler");
    Arc::new(engine)
}

async fn enqueue(engine: &Engine) -> Uuid {
    engine
        .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), 0)
        .await
        .expect("enqueue")
        .id
}

/// Pick the next run the way a worker does and check it is `run_id`.
async fn pick(store: &InMemoryStore, run_id: Uuid) {
    let picked = store
        .pick_next_pending(None)
        .await
        .expect("pick")
        .expect("a pending run");
    assert_eq!(picked.id, run_id);
}

fn single() -> Single {
    Single {
        op: CountingOp::new(),
    }
}

/// Whether a worker would find nothing to pick.
async fn nothing_to_pick(store: &InMemoryStore) -> bool {
    let picked = store.pick_next_pending(None).await.expect("pick");
    picked.is_none()
}

async fn status_of(store: &InMemoryStore, run_id: Uuid) -> RunStatus {
    store
        .get_run(run_id)
        .await
        .expect("get run")
        .expect("run exists")
        .status
        .state
}

#[tokio::test]
async fn pause_pending_run_then_resume_returns_to_pending() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, single());
        let run_id = enqueue(&engine).await;

        let pause = engine.pause_run(run_id).await.expect("pause");
        assert_eq!(pause.run.status.state, RunStatus::Paused);
        assert_eq!(pause.run.resume_status, Some(RunStatus::Pending));
        assert!(pause.paused_descendants.is_empty());

        // A paused run is never picked.
        assert!(nothing_to_pick(&store).await);

        let resume = engine.resume_paused_run(run_id).await.expect("resume");
        assert_eq!(resume.run.status.state, RunStatus::Pending);
        assert_eq!(resume.run.resume_status, None);
        assert!(resume.resumed_descendants.is_empty());

        pick(&store, run_id).await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_running_run_interrupts_step_and_resume_replays() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let gate = GateOp::new();
        let after = CountingOp::new();
        let engine = engine_with(
            &store,
            GateThenCount {
                gate: gate.clone(),
                after: after.clone(),
            },
        );
        let run_id = enqueue(&engine).await;
        pick(&store, run_id).await;

        let worker = engine.clone();
        let task = spawn(async move { worker.execute_handler_run(run_id).await });
        gate.entered.notified().await;

        let pause = engine.pause_run(run_id).await.expect("pause");
        assert_eq!(pause.run.status.state, RunStatus::Paused);
        assert_eq!(pause.run.resume_status, Some(RunStatus::Running));

        // The step in flight is interrupted by the pause itself, before the
        // execution notices it.
        let steps = store.list_steps(run_id).await.expect("steps");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].status.state, StepStatus::Failed);
        assert_eq!(steps[0].error.as_deref(), Some(STEP_INTERRUPTED_ERROR));
        gate.release.notify_one();

        // The interrupted step cannot complete, the next one is never started
        // and the run is neither failed nor completed.
        let result = task.await.expect("task").expect("execution");
        assert_eq!(result.run.status.state, RunStatus::Paused);
        assert_eq!(after.calls(), 0);
        let steps = store.list_steps(run_id).await.expect("steps");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].status.state, StepStatus::Failed);
        assert_eq!(steps[0].error.as_deref(), Some(STEP_INTERRUPTED_ERROR));

        let resume = engine.resume_paused_run(run_id).await.expect("resume");
        assert_eq!(resume.run.status.state, RunStatus::Pending);
        assert_eq!(resume.run.resume_status, None);

        // `notify_one` stores a permit: the gate executed again goes through.
        gate.release.notify_one();
        pick(&store, run_id).await;
        let result = engine
            .execute_handler_run(run_id)
            .await
            .expect("resumed run");
        assert_eq!(result.run.status.state, RunStatus::Completed);
        // The interrupted step is executed again, then the next one.
        assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
        assert_eq!(after.calls(), 1);
        // The interrupted record stays in the history next to the new one.
        let steps = store.list_steps(run_id).await.expect("steps");
        let gates: Vec<_> = steps.iter().filter(|s| s.name == "gate").collect();
        assert_eq!(gates.len(), 2);
        let interrupted = gates
            .iter()
            .filter(|s| s.error.as_deref() == Some(STEP_INTERRUPTED_ERROR))
            .count();
        assert_eq!(interrupted, 1);
        let completed = gates
            .iter()
            .filter(|s| s.status.state == StepStatus::Completed)
            .count();
        assert_eq!(completed, 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_completed_run_is_rejected() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, single());
        let run_id = enqueue(&engine).await;
        pick(&store, run_id).await;
        engine.execute_handler_run(run_id).await.expect("run");

        let err = engine.pause_run(run_id).await.expect_err("completed run");
        assert!(matches!(
            err,
            EngineError::Store(StoreError::InvalidTransition {
                from: RunStatus::Completed,
                to: RunStatus::Paused,
            })
        ));
        assert_eq!(status_of(&store, run_id).await, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_already_paused_run_is_rejected() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, single());
        let run_id = enqueue(&engine).await;
        engine.pause_run(run_id).await.expect("pause");

        let err = engine.pause_run(run_id).await.expect_err("already paused");
        assert!(matches!(
            err,
            EngineError::Store(StoreError::InvalidTransition {
                from: RunStatus::Paused,
                ..
            })
        ));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_unknown_run_is_not_found() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, single());
        let unknown = Uuid::now_v7();

        let err = engine.pause_run(unknown).await.expect_err("unknown run");
        assert!(matches!(err, EngineError::Store(StoreError::RunNotFound(id)) if id == unknown));
        let err = engine
            .resume_paused_run(unknown)
            .await
            .expect_err("unknown run");
        assert!(matches!(err, EngineError::Store(StoreError::RunNotFound(id)) if id == unknown));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_resume_of_a_run_that_is_not_paused_is_rejected() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, single());
        let run_id = enqueue(&engine).await;

        let err = engine
            .resume_paused_run(run_id)
            .await
            .expect_err("pending run");
        assert!(matches!(
            err,
            EngineError::Store(StoreError::InvalidTransition {
                from: RunStatus::Pending,
                ..
            })
        ));
        assert_eq!(status_of(&store, run_id).await, RunStatus::Pending);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_parent_pauses_child_chain_and_child_alone_is_rejected() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let mut engine = new_engine(&store);
        engine.register(ApprovalChild).expect("register child");
        engine.register(ApprovalParent).expect("register parent");
        let engine = Arc::new(engine);

        let run_id = enqueue(&engine).await;
        pick(&store, run_id).await;
        let result = engine.execute_handler_run(run_id).await.expect("run");
        assert_eq!(result.run.status.state, RunStatus::AwaitingApproval);
        let children = store
            .list_active_descendants(run_id)
            .await
            .expect("descendants");
        assert_eq!(children.len(), 1);
        let child_id = children[0].id;

        let err = engine.pause_run(child_id).await.expect_err("child run");
        assert!(matches!(
            err,
            EngineError::ChildRunNotPausable { run_id: id, root_run_id }
                if id == child_id && root_run_id == run_id
        ));
        let err = engine
            .resume_paused_run(child_id)
            .await
            .expect_err("child run");
        assert!(matches!(err, EngineError::ChildRunNotPausable { .. }));
        assert_eq!(
            status_of(&store, child_id).await,
            RunStatus::AwaitingApproval
        );

        let pause = engine.pause_run(run_id).await.expect("pause");
        assert_eq!(pause.paused_descendants, vec![child_id]);
        let child = store
            .get_run(child_id)
            .await
            .expect("get child")
            .expect("child exists");
        assert_eq!(child.status.state, RunStatus::Paused);
        assert_eq!(child.resume_status, Some(RunStatus::AwaitingApproval));

        let resume = engine.resume_paused_run(run_id).await.expect("resume");
        assert_eq!(resume.run.status.state, RunStatus::AwaitingApproval);
        assert_eq!(resume.resumed_descendants, vec![child_id]);
        assert_eq!(
            status_of(&store, child_id).await,
            RunStatus::AwaitingApproval
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_running_parent_stops_child_and_resume_finishes_the_chain() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let child = GatedChild {
            gate: GateOp::new(),
            after: CountingOp::new(),
        };
        let parent_after = CountingOp::new();
        let mut engine = new_engine(&store);
        engine.register(child.clone()).expect("register child");
        engine
            .register(GatedParent {
                child: child.clone(),
                after: parent_after.clone(),
            })
            .expect("register parent");
        let engine = Arc::new(engine);

        let run_id = enqueue(&engine).await;
        pick(&store, run_id).await;
        let worker = engine.clone();
        let task = spawn(async move { worker.execute_handler_run(run_id).await });
        child.gate.entered.notified().await;

        let pause = engine.pause_run(run_id).await.expect("pause");
        assert_eq!(pause.paused_descendants.len(), 1);
        let child_id = pause.paused_descendants[0];
        child.gate.release.notify_one();

        let result = task.await.expect("task").expect("execution");
        assert_eq!(result.run.status.state, RunStatus::Paused);
        assert_eq!(status_of(&store, child_id).await, RunStatus::Paused);
        assert_eq!(child.after.calls(), 0);
        assert_eq!(parent_after.calls(), 0);
        // Both the parent's `Workflow` step and the child's step in flight are
        // interrupted: the resumed parent re-enters the same child run.
        let parent_steps = store.list_steps(run_id).await.expect("steps");
        assert_eq!(parent_steps.len(), 1);
        assert_eq!(parent_steps[0].status.state, StepStatus::Failed);
        assert_eq!(
            parent_steps[0].error.as_deref(),
            Some(STEP_INTERRUPTED_ERROR)
        );
        let child_steps = store.list_steps(child_id).await.expect("child steps");
        assert_eq!(child_steps.len(), 1);
        assert_eq!(
            child_steps[0].error.as_deref(),
            Some(STEP_INTERRUPTED_ERROR)
        );

        engine.resume_paused_run(run_id).await.expect("resume");
        assert_eq!(status_of(&store, run_id).await, RunStatus::Pending);
        // The child was executing: it stays `Running` for its root's replay.
        assert_eq!(status_of(&store, child_id).await, RunStatus::Running);

        // `notify_one` stores a permit: the gate executed again goes through.
        child.gate.release.notify_one();
        pick(&store, run_id).await;
        let result = engine
            .execute_handler_run(run_id)
            .await
            .expect("resumed run");
        assert_eq!(result.run.status.state, RunStatus::Completed);
        assert_eq!(status_of(&store, child_id).await, RunStatus::Completed);
        assert_eq!(child.gate.calls.load(Ordering::SeqCst), 2);
        assert_eq!(child.after.calls(), 1);
        assert_eq!(parent_after.calls(), 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_sleeping_run_resumes_to_sleeping_with_its_deadline() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, Sleeper);
        let run_id = enqueue(&engine).await;
        pick(&store, run_id).await;
        let result = engine.execute_handler_run(run_id).await.expect("run");
        assert_eq!(result.run.status.state, RunStatus::Sleeping);
        let deadline = result.run.scheduled_at;
        assert!(deadline.is_some());

        let pause = engine.pause_run(run_id).await.expect("pause");
        assert_eq!(pause.run.resume_status, Some(RunStatus::Sleeping));
        assert_eq!(pause.run.scheduled_at, deadline);

        let resume = engine.resume_paused_run(run_id).await.expect("resume");
        assert_eq!(resume.run.status.state, RunStatus::Sleeping);
        assert_eq!(resume.run.scheduled_at, deadline);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_sleeping_run_resumes_to_pending_once_its_deadline_passed() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, Sleeper);
        let run_id = enqueue(&engine).await;
        pick(&store, run_id).await;
        let result = engine.execute_handler_run(run_id).await.expect("run");
        assert_eq!(result.run.status.state, RunStatus::Sleeping);

        engine.pause_run(run_id).await.expect("pause");
        // The deadline passes while the run is paused.
        store
            .update_run(
                run_id,
                RunUpdate {
                    scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                    ..RunUpdate::default()
                },
            )
            .await
            .expect("move the deadline");
        // The waker never claims a paused run.
        let woken = store.claim_due_sleeping_runs(10).await.expect("claim");
        assert!(woken.is_empty());
        assert_eq!(status_of(&store, run_id).await, RunStatus::Paused);

        let resume = engine.resume_paused_run(run_id).await.expect("resume");
        assert_eq!(resume.run.status.state, RunStatus::Pending);
        assert_eq!(resume.run.resume_status, None);

        // Due at once: picked and completed past its delay.
        pick(&store, run_id).await;
        let result = engine
            .execute_handler_run(run_id)
            .await
            .expect("resumed run");
        assert_eq!(result.run.status.state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_workflow_holds_its_runs_until_resumed() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, single());

        let pause = engine.pause_workflow(WORKFLOW, None).await.expect("pause");
        assert_eq!(pause.workflow_name, WORKFLOW);
        let again = engine
            .pause_workflow(WORKFLOW, None)
            .await
            .expect("pause again");
        assert_eq!(again.paused_at, pause.paused_at);

        // Runs are still created, but not picked.
        let run_id = enqueue(&engine).await;
        assert!(nothing_to_pick(&store).await);
        assert_eq!(status_of(&store, run_id).await, RunStatus::Pending);

        assert!(engine.resume_workflow(WORKFLOW).await.expect("resume"));
        assert!(
            !engine
                .resume_workflow(WORKFLOW)
                .await
                .expect("resume again")
        );
        pick(&store, run_id).await;
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_unknown_workflow_is_rejected() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, single());

        let err = engine
            .pause_workflow("no-such-workflow", None)
            .await
            .expect_err("unknown workflow");
        assert!(matches!(err, EngineError::InvalidWorkflow(_)));
        let err = engine
            .resume_workflow("no-such-workflow")
            .await
            .expect_err("unknown workflow");
        assert!(matches!(err, EngineError::InvalidWorkflow(_)));
        let pauses = store.list_workflow_pauses().await.expect("list");
        assert!(pauses.is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_then_cancel_stops_the_run_for_good() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(&store, single());
        let run_id = enqueue(&engine).await;
        engine.pause_run(run_id).await.expect("pause");

        let cancellation = engine.cancel_run(run_id).await.expect("cancel");
        assert_eq!(cancellation.run.status.state, RunStatus::Cancelled);
        assert_eq!(cancellation.run.resume_status, None);

        let err = engine
            .resume_paused_run(run_id)
            .await
            .expect_err("cancelled run");
        assert!(matches!(
            err,
            EngineError::Store(StoreError::InvalidTransition {
                from: RunStatus::Cancelled,
                ..
            })
        ));
    })
    .await
    .expect("test timed out");
}
