//! Pause and resume of a run executing in this process.
//!
//! Every test drives a real [`Engine`] over a real [`InMemoryStore`], under
//! [`ExecutionMode::Local`]: a resumed run restarts in a background task, next
//! to the execution that may still be inside its step. Test names start with
//! `pause_local_` so `cargo test -p ironflow-engine --test pause_local`
//! selects them.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::spawn;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::error::OperationError;
use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::operation::{Operation, OperationContext};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store and spawn tasks.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name of every handler in this file.
const WORKFLOW: &str = "pause-local";

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
/// the test pauses and resumes the run while this step is in flight.
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

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
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

/// A single gated step.
struct GateOnly {
    gate: GateOp,
}

impl WorkflowHandler for GateOnly {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("gate", &self.gate).await?;
            Ok(())
        })
    }
}

fn engine_with(store: &Arc<InMemoryStore>, handler: impl WorkflowHandler + 'static) -> Arc<Engine> {
    let store: Arc<dyn Store> = store.clone();
    let provider: Arc<dyn AgentProvider> = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store, provider).with_execution_mode(ExecutionMode::Local);
    engine.register(handler).expect("register handler");
    Arc::new(engine)
}

/// Create a run and pick it the way the execution of a started run would.
async fn start(engine: &Engine, store: &InMemoryStore) -> Uuid {
    let run_id = engine
        .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), 0)
        .await
        .expect("enqueue")
        .id;
    let picked = store
        .pick_next_pending(None)
        .await
        .expect("pick")
        .expect("a pending run");
    assert_eq!(picked.id, run_id);
    run_id
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

/// Wait until the run reaches `expected`.
async fn wait_for_status(store: &InMemoryStore, run_id: Uuid, expected: RunStatus) {
    while status_of(store, run_id).await != expected {
        sleep(Duration::from_millis(10)).await;
    }
}

/// How many steps named `name` ended `Completed`.
async fn completed_steps(store: &InMemoryStore, run_id: Uuid, name: &str) -> usize {
    store
        .list_steps(run_id)
        .await
        .expect("steps")
        .iter()
        .filter(|s| s.name == name && s.status.state == StepStatus::Completed)
        .count()
}

#[tokio::test]
async fn pause_local_resume_before_step_boundary_runs_one_handler() {
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
        let run_id = start(&engine, &store).await;

        let worker = engine.clone();
        let task = spawn(async move { worker.execute_handler_run(run_id).await });
        gate.entered.notified().await;

        engine.pause_run(run_id).await.expect("pause");
        let resume = engine.resume_paused_run(run_id).await.expect("resume");
        assert_ne!(resume.run.status.state, RunStatus::Paused);
        gate.release.notify_one();

        wait_for_status(&store, run_id, RunStatus::Completed).await;
        let result = task.await.expect("task").expect("execution");
        assert_eq!(result.run.status.state, RunStatus::Completed);

        assert_eq!(gate.calls(), 1);
        assert_eq!(after.calls(), 1);
        assert_eq!(completed_steps(&store, run_id, "gate").await, 1);
        assert_eq!(completed_steps(&store, run_id, "after").await, 1);
        assert_eq!(status_of(&store, run_id).await, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_local_resume_after_boundary_still_restarts_once() {
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
        let run_id = start(&engine, &store).await;

        let worker = engine.clone();
        let task = spawn(async move { worker.execute_handler_run(run_id).await });
        gate.entered.notified().await;

        engine.pause_run(run_id).await.expect("pause");
        gate.release.notify_one();

        // The execution stops on the pause: the run stays paused.
        let result = task.await.expect("task").expect("execution");
        assert_eq!(result.run.status.state, RunStatus::Paused);
        assert_eq!(status_of(&store, run_id).await, RunStatus::Paused);
        assert_eq!(after.calls(), 0);

        // The interrupted step is executed again by the restart, once.
        engine.resume_paused_run(run_id).await.expect("resume");
        gate.release.notify_one();

        wait_for_status(&store, run_id, RunStatus::Completed).await;
        assert_eq!(gate.calls(), 2);
        assert_eq!(after.calls(), 1);
        assert_eq!(completed_steps(&store, run_id, "gate").await, 1);
        assert_eq!(completed_steps(&store, run_id, "after").await, 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pause_local_resume_when_last_step_is_slow_completes_once() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let gate = GateOp::new();
        let engine = engine_with(&store, GateOnly { gate: gate.clone() });
        let run_id = start(&engine, &store).await;

        let worker = engine.clone();
        let task = spawn(async move { worker.execute_handler_run(run_id).await });
        gate.entered.notified().await;

        engine.pause_run(run_id).await.expect("pause");
        engine.resume_paused_run(run_id).await.expect("resume");
        gate.release.notify_one();

        let result = task.await.expect("task").expect("execution");
        assert_eq!(result.run.status.state, RunStatus::Completed);

        // Give a wrongly started second execution the time to show itself.
        sleep(Duration::from_millis(200)).await;
        assert_eq!(status_of(&store, run_id).await, RunStatus::Completed);
        assert_eq!(gate.calls(), 1);
        assert_eq!(completed_steps(&store, run_id, "gate").await, 1);
    })
    .await
    .expect("test timed out");
}
