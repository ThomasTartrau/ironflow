//! Non-regression tests: a run requeued by the reaper after its worker lost
//! the lease must resume where it stopped instead of executing its finished
//! steps again.
//!
//! A worker crash is simulated for real: the run is picked with a lease that
//! expires at once, executed on a spawned task, and the task is aborted while
//! a step hangs. The lease is then reaped and the run's open steps are
//! interrupted, like `ironflow-api`'s reaper does, before another execution
//! picks the run up. Every test drives a real [`Engine`] over a real
//! [`InMemoryStore`]. Test names start with `lease_lost_` so `cargo test -p
//! ironflow-engine lease_lost` selects them.

use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::spawn;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::error::OperationError;
use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::HumanInputConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::operation::{Operation, OperationContext};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    LeaseRequest, ReapedRun, RunFilter, RunStatus, RunUpdate, Step, StepKind, StepStatus,
    StepUpdate, TriggerKind,
};
use ironflow_store::store::{LEASE_EXPIRED_ERROR, RunStore, STEP_INTERRUPTED_ERROR, Store};

/// Test timeout for bodies that touch the store and spawn tasks.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name of every top-level handler in this file.
const WORKFLOW: &str = "lease-lost";

/// Workflow name of [`Child`].
const CHILD_WORKFLOW: &str = "lease-lost-child";

/// Worker id holding the lease that is lost.
const WORKER: &str = "worker-that-dies";

/// A custom operation that counts how many times it actually ran.
#[derive(Clone)]
struct CountingOp {
    calls: Arc<AtomicU32>,
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

/// A custom operation that counts its calls, then never returns while
/// `hang` is set: the worker running it is killed in the middle of the step.
#[derive(Clone)]
struct HangingOp {
    calls: Arc<AtomicU32>,
    hang: Arc<AtomicBool>,
}

#[async_trait]
impl Operation for HangingOp {
    fn kind(&self) -> &str {
        "hanging-op"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.hang.load(Ordering::SeqCst) {
            pending::<()>().await;
        }
        Ok(json!({"hung": false}))
    }
}

fn counting() -> CountingOp {
    CountingOp {
        calls: Arc::new(AtomicU32::new(0)),
    }
}

fn hanging() -> HangingOp {
    HangingOp {
        calls: Arc::new(AtomicU32::new(0)),
        hang: Arc::new(AtomicBool::new(true)),
    }
}

/// Three operation steps, the second one hanging. Fails once with a
/// transient error after the third step while `fail_once` is set, and
/// reports `version` as its handler version.
struct ThreeSteps {
    one: CountingOp,
    two: HangingOp,
    three: CountingOp,
    fail_once: Arc<AtomicBool>,
    version: &'static str,
}

impl ThreeSteps {
    fn new() -> Self {
        Self {
            one: counting(),
            two: hanging(),
            three: counting(),
            fail_once: Arc::new(AtomicBool::new(false)),
            version: "1",
        }
    }
}

impl WorkflowHandler for ThreeSteps {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn version(&self) -> Option<&str> {
        Some(self.version)
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("one", &self.one).await?;
            ctx.operation("two", &self.two).await?;
            ctx.operation("three", &self.three).await?;
            if self.fail_once.swap(false, Ordering::SeqCst) {
                return Err(EngineError::Operation(OperationError::Http {
                    status: Some(503),
                    message: "upstream unavailable".to_string(),
                }));
            }
            Ok(())
        })
    }
}

/// The typed answer [`SkipThenInput`] asks for.
#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// A skipped step, a human input, then a hanging step.
struct SkipThenInput {
    last: HangingOp,
}

impl WorkflowHandler for SkipThenInput {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.skip("maybe", "not needed").await?;
            let answers = ctx
                .human_input::<Answers>("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            assert_eq!(answers.answers, vec!["ok".to_string()]);
            ctx.operation("last", &self.last).await?;
            Ok(())
        })
    }
}

/// Input of [`Child`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ChildInput {}

/// A child workflow: a counting step, then a hanging step.
#[derive(Clone)]
struct Child {
    count: CountingOp,
    hang: HangingOp,
}

impl WorkflowHandler for Child {
    fn name(&self) -> &str {
        CHILD_WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("child-count", &self.count).await?;
            ctx.operation("child-hang", &self.hang).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Child {
    type Input = ChildInput;
}

/// A counting step, then the [`Child`] sub-workflow.
struct Parent {
    count: CountingOp,
    child: Child,
}

impl WorkflowHandler for Parent {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("parent-count", &self.count).await?;
            ctx.workflow(&self.child, ChildInput {}).await?;
            Ok(())
        })
    }
}

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(ClaudeCodeProvider::new())
}

fn new_engine(store: &Arc<InMemoryStore>) -> Engine {
    let store: Arc<dyn Store> = store.clone();
    Engine::new(store, provider())
}

fn engine_with(store: &Arc<InMemoryStore>, handler: impl WorkflowHandler + 'static) -> Arc<Engine> {
    let mut engine = new_engine(store);
    engine.register(handler).expect("register handler");
    Arc::new(engine)
}

async fn enqueue(engine: &Engine, max_retries: u32) -> Uuid {
    engine
        .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), max_retries)
        .await
        .expect("enqueue")
        .id
}

/// Pick the run the way a worker does, with a lease that expires at once.
async fn pick_with_expiring_lease(store: &InMemoryStore, run_id: Uuid) {
    let picked = store
        .pick_next_pending(Some(LeaseRequest {
            worker_id: WORKER.to_string(),
            ttl: Duration::from_millis(1),
        }))
        .await
        .expect("pick")
        .expect("a pending run");
    assert_eq!(picked.id, run_id);
}

/// Execute the run on a task and kill it once `hang` has been entered
/// `reached` times in total.
async fn crash_inside(engine: &Arc<Engine>, run_id: Uuid, hang: &HangingOp, reached: u32) {
    let worker = engine.clone();
    let task = spawn(async move { worker.execute_handler_run(run_id).await });
    while hang.calls.load(Ordering::SeqCst) < reached {
        sleep(Duration::from_millis(5)).await;
    }
    task.abort();
    let err = task.await.expect_err("the worker was killed");
    assert!(err.is_cancelled());
}

/// Reap the expired lease and clean the run's steps up like the reaper.
async fn recover(engine: &Engine, store: &InMemoryStore, run_id: Uuid) -> ReapedRun {
    sleep(Duration::from_millis(5)).await;
    let reaped = store
        .reap_expired_leases(100)
        .await
        .expect("reap")
        .into_iter()
        .find(|r| r.run.id == run_id)
        .expect("the run's lease expired");
    if reaped.to == RunStatus::Pending {
        engine
            .interrupt_running_steps(run_id)
            .await
            .expect("interrupt running steps");
    } else {
        engine
            .fail_orphaned_steps(run_id, LEASE_EXPIRED_ERROR)
            .await
            .expect("fail orphaned steps");
    }
    reaped
}

/// Steps of `run_id` called `name`.
async fn steps_named(store: &InMemoryStore, run_id: Uuid, name: &str) -> Vec<Step> {
    store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .filter(|s| s.name == name)
        .collect()
}

fn failed_with(step: &Step, error: &str) -> bool {
    step.status.state == StepStatus::Failed && step.error.as_deref() == Some(error)
}

fn is_interrupted(step: &Step) -> bool {
    failed_with(step, STEP_INTERRUPTED_ERROR)
}

#[tokio::test]
async fn lease_lost_run_does_not_rerun_completed_steps() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let handler = ThreeSteps::new();
        let one = handler.one.clone();
        let two = handler.two.clone();
        let three = handler.three.clone();
        let engine = engine_with(&store, handler);

        let run_id = enqueue(&engine, 3).await;
        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &two, 1).await;

        let reaped = recover(&engine, &store, run_id).await;
        assert_eq!(reaped.to, RunStatus::Pending);
        assert_eq!(reaped.run.retry_count, 0);
        assert_eq!(reaped.run.lease_recoveries, 1);

        two.hang.store(false, Ordering::SeqCst);
        pick_with_expiring_lease(&store, run_id).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the requeued run completes");

        assert_eq!(
            one.calls.load(Ordering::SeqCst),
            1,
            "step one must be replayed"
        );
        assert_eq!(two.calls.load(Ordering::SeqCst), 2);
        assert_eq!(three.calls.load(Ordering::SeqCst), 1);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
        assert_eq!(run.retry_count, 0);
        assert_eq!(run.lease_recoveries, 1);

        let steps = store.list_steps(run_id).await.expect("list steps");
        assert!(steps.iter().all(|s| s.attempt == 1), "steps: {steps:?}");
        assert_eq!(steps_named(&store, run_id, "one").await.len(), 1);
        assert_eq!(steps_named(&store, run_id, "three").await.len(), 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn lease_lost_marks_running_step_failed_and_reexecutes_it() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let handler = ThreeSteps::new();
        let two = handler.two.clone();
        let engine = engine_with(&store, handler);

        let run_id = enqueue(&engine, 3).await;
        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &two, 1).await;
        recover(&engine, &store, run_id).await;

        let interrupted = steps_named(&store, run_id, "two").await;
        assert_eq!(interrupted.len(), 1);
        assert!(
            is_interrupted(&interrupted[0]),
            "step: {:?}",
            interrupted[0]
        );
        assert!(interrupted[0].completed_at.is_some());

        two.hang.store(false, Ordering::SeqCst);
        pick_with_expiring_lease(&store, run_id).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the requeued run completes");

        let records = steps_named(&store, run_id, "two").await;
        assert_eq!(records.len(), 2, "steps: {records:?}");
        let position = interrupted[0].position;
        assert!(records.iter().all(|s| s.position == position));
        assert_eq!(records.iter().filter(|s| is_interrupted(s)).count(), 1);
        assert_eq!(
            records
                .iter()
                .filter(|s| s.status.state == StepStatus::Completed)
                .count(),
            1
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn lease_lost_replays_skipped_and_human_input_steps() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let op = hanging();
        let engine = engine_with(&store, SkipThenInput { last: op.clone() });

        let run_id = enqueue(&engine, 3).await;
        store
            .pick_next_pending(None)
            .await
            .expect("pick")
            .expect("a pending run");
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the handler suspends on the input");

        let input = steps_named(&store, run_id, "clarify").await;
        assert_eq!(input.len(), 1);
        assert_eq!(input[0].status.state, StepStatus::AwaitingApproval);
        store
            .update_step(
                input[0].id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(json!({"answers": ["ok"]})),
                    completed_at: Some(Utc::now()),
                    clear_approval_deadline: true,
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("store the answer");
        store
            .update_run_status(run_id, RunStatus::Pending)
            .await
            .expect("requeue the answered run");

        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &op, 1).await;
        let reaped = recover(&engine, &store, run_id).await;
        assert_eq!(reaped.to, RunStatus::Pending);

        op.hang.store(false, Ordering::SeqCst);
        pick_with_expiring_lease(&store, run_id).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the requeued run completes");

        assert_eq!(steps_named(&store, run_id, "maybe").await.len(), 1);
        let input = steps_named(&store, run_id, "clarify").await;
        assert_eq!(input.len(), 1, "the input must not be asked again");
        assert_eq!(input[0].status.state, StepStatus::Completed);
        assert_eq!(op.calls.load(Ordering::SeqCst), 2);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn lease_lost_reenters_same_child_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let child = Child {
            count: counting(),
            hang: hanging(),
        };
        let parent = Parent {
            count: counting(),
            child: child.clone(),
        };
        let parent_count = parent.count.clone();
        let mut engine = new_engine(&store);
        engine.register(child.clone()).expect("register child");
        engine.register(parent).expect("register parent");
        let engine = Arc::new(engine);

        let run_id = enqueue(&engine, 3).await;
        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &child.hang, 1).await;
        assert_eq!(
            recover(&engine, &store, run_id).await.to,
            RunStatus::Pending
        );

        // The second worker dies inside the re-entered child too.
        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &child.hang, 2).await;
        assert_eq!(
            recover(&engine, &store, run_id).await.to,
            RunStatus::Pending
        );

        child.hang.hang.store(false, Ordering::SeqCst);
        pick_with_expiring_lease(&store, run_id).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the requeued run completes");

        let children = store
            .list_runs(
                RunFilter {
                    workflow_name: Some(CHILD_WORKFLOW.to_string()),
                    ..RunFilter::default()
                },
                1,
                10,
            )
            .await
            .expect("list child runs");
        assert_eq!(children.items.len(), 1, "a single child run must exist");
        let child_run = &children.items[0];
        assert_eq!(child_run.status.state, RunStatus::Completed);

        assert_eq!(parent_count.calls.load(Ordering::SeqCst), 1);
        assert_eq!(child.count.calls.load(Ordering::SeqCst), 1);
        assert_eq!(child.hang.calls.load(Ordering::SeqCst), 3);

        let workflow_steps: Vec<Step> = store
            .list_steps(run_id)
            .await
            .expect("list steps")
            .into_iter()
            .filter(|s| s.kind == StepKind::Workflow)
            .collect();
        assert_eq!(workflow_steps.len(), 3, "steps: {workflow_steps:?}");
        let interrupted: Vec<&Step> = workflow_steps
            .iter()
            .filter(|s| is_interrupted(s))
            .collect();
        assert_eq!(interrupted.len(), 2);
        for step in interrupted {
            let recorded = step
                .output
                .as_ref()
                .and_then(|o| o.get("child_run_id"))
                .and_then(Value::as_str)
                .expect("the interrupted step records its child run");
            assert_eq!(recorded, child_run.id.to_string());
        }

        let hang_steps = steps_named(&store, child_run.id, "child-hang").await;
        assert_eq!(hang_steps.len(), 3, "steps: {hang_steps:?}");
        assert_eq!(hang_steps.iter().filter(|s| is_interrupted(s)).count(), 2);
        let count_steps = steps_named(&store, child_run.id, "child-count").await;
        assert_eq!(count_steps.len(), 1);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
        assert_eq!(run.lease_recoveries, 2);
        assert_eq!(run.retry_count, 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn lease_lost_beyond_max_retries_fails_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let handler = ThreeSteps::new();
        let two = handler.two.clone();
        let engine = engine_with(&store, handler);

        let run_id = enqueue(&engine, 1).await;
        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &two, 1).await;
        assert_eq!(
            recover(&engine, &store, run_id).await.to,
            RunStatus::Pending
        );

        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &two, 2).await;
        let reaped = recover(&engine, &store, run_id).await;
        assert_eq!(reaped.to, RunStatus::Failed);
        assert_eq!(reaped.run.lease_recoveries, 2);
        assert_eq!(reaped.run.retry_count, 0);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some(LEASE_EXPIRED_ERROR));

        let records = steps_named(&store, run_id, "two").await;
        assert_eq!(records.len(), 2);
        assert_eq!(records.iter().filter(|s| is_interrupted(s)).count(), 1);
        let expired = records
            .iter()
            .filter(|s| failed_with(s, LEASE_EXPIRED_ERROR))
            .count();
        assert_eq!(expired, 1);

        let picked = store.pick_next_pending(None).await.expect("pick");
        assert!(
            picked.is_none(),
            "an exhausted run must not be picked again"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn lease_lost_handler_retry_starts_new_attempt_without_replay() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let handler = ThreeSteps::new();
        handler.fail_once.store(true, Ordering::SeqCst);
        let one = handler.one.clone();
        let two = handler.two.clone();
        let three = handler.three.clone();
        let engine = engine_with(&store, handler);

        let run_id = enqueue(&engine, 3).await;
        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &two, 1).await;
        recover(&engine, &store, run_id).await;

        two.hang.store(false, Ordering::SeqCst);
        pick_with_expiring_lease(&store, run_id).await;
        assert!(engine.execute_handler_run(run_id).await.is_err());

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Retrying);
        assert_eq!(run.retry_count, 1);
        assert_eq!(run.lease_recoveries, 1);
        assert_eq!(one.calls.load(Ordering::SeqCst), 1);

        // Skip the backoff, then pick the retry.
        store
            .update_run(
                run_id,
                RunUpdate {
                    scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                    ..RunUpdate::default()
                },
            )
            .await
            .expect("rewind scheduled_at");
        let picked = store
            .pick_next_pending(None)
            .await
            .expect("pick")
            .expect("a run waiting for its retry");
        assert_eq!(picked.id, run_id);
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the retry completes");

        assert_eq!(
            one.calls.load(Ordering::SeqCst),
            2,
            "a new attempt replays nothing"
        );
        assert_eq!(two.calls.load(Ordering::SeqCst), 3);
        assert_eq!(three.calls.load(Ordering::SeqCst), 2);

        let steps = store.list_steps(run_id).await.expect("list steps");
        assert_eq!(steps.iter().filter(|s| s.attempt == 2).count(), 3);
        assert_eq!(steps.iter().filter(|s| s.attempt == 1).count(), 4);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
        assert_eq!(run.retry_count, 1);
        assert_eq!(run.lease_recoveries, 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn lease_lost_incompatible_handler_version_replays_nothing() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let v1 = ThreeSteps::new();
        let one = v1.one.clone();
        let two = v1.two.clone();
        let three = v1.three.clone();
        let engine1 = engine_with(&store, v1);

        let run_id = enqueue(&engine1, 3).await;
        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine1, run_id, &two, 1).await;
        recover(&engine1, &store, run_id).await;
        let steps_before = store.list_steps(run_id).await.expect("list steps").len();

        // The worker that picks the run up again runs a redeployed handler.
        let v2 = ThreeSteps {
            one: one.clone(),
            two: two.clone(),
            three: three.clone(),
            fail_once: Arc::new(AtomicBool::new(false)),
            version: "2",
        };
        two.hang.store(false, Ordering::SeqCst);
        let engine2 = engine_with(&store, v2);
        pick_with_expiring_lease(&store, run_id).await;
        let err = engine2
            .execute_handler_run(run_id)
            .await
            .expect_err("an incompatible handler version must refuse the resume");
        assert!(
            matches!(err, EngineError::HandlerVersionMismatch { .. }),
            "unexpected error: {err:?}"
        );

        assert_eq!(one.calls.load(Ordering::SeqCst), 1);
        assert_eq!(two.calls.load(Ordering::SeqCst), 1);
        assert_eq!(three.calls.load(Ordering::SeqCst), 0);
        let steps_after = store.list_steps(run_id).await.expect("list steps").len();
        assert_eq!(steps_before, steps_after, "no step must be created");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.lease_recoveries, 1);
        assert_eq!(run.retry_count, 0);
    })
    .await
    .expect("test timed out");
}
