//! Integration tests for the typed output of a run.
//!
//! A handler calls `ctx.set_output(&value)`; the engine persists the value on
//! the run when the execution ends, and a parent reads the output of a child
//! through `SubWorkflowOutput::output::<T>()`. Every test drives a real
//! [`Engine`] over a real [`InMemoryStore`]. Test names start with
//! `set_output` so `cargo test -p ironflow-engine set_output` selects them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, to_value};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{DelayConfig, WorkflowOptions};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::executor::SubWorkflowOutcome;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::plan::PlanOptions;
use ironflow_engine::wake::RunWaker;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{Run, RunFilter, RunStatus, RunUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for every body.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The output every reviewer in this file sets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Verdict {
    approved: bool,
    score: u8,
}

/// Input of [`Reviewer`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ReviewInput {
    approved: bool,
    /// Return an error after setting the output.
    fail: bool,
}

/// Payload of a workflow that takes nothing.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoInput {}

/// What the parents read back from their child, shared with the test.
type Seen = Arc<Mutex<Vec<Result<Option<Verdict>, String>>>>;

fn seen(seen: &Seen) -> Vec<Result<Option<Verdict>, String>> {
    seen.lock().expect("seen lock").clone()
}

/// Record what `read` returned: the typed output, or the kind of error.
fn record(seen: &Seen, read: Result<Option<Verdict>, EngineError>) {
    let entry = read.map_err(|err| match err {
        EngineError::Serialization(_) => "serialization".to_string(),
        other => other.to_string(),
    });
    seen.lock().expect("seen lock").push(entry);
}

fn verdict(approved: bool) -> Verdict {
    Verdict {
        approved,
        score: if approved { 9 } else { 2 },
    }
}

/// A child that sets a [`Verdict`] as its output, then fails if asked to.
struct Reviewer;

impl WorkflowHandler for Reviewer {
    fn name(&self) -> &str {
        "reviewer"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: ReviewInput = ctx.input().await?;
            ctx.set_output(&verdict(input.approved))?;
            if input.fail {
                return Err(EngineError::InvalidWorkflow(
                    "the review found blocking issues".to_string(),
                ));
            }
            Ok(())
        })
    }
}

impl TypedWorkflow for Reviewer {
    type Input = ReviewInput;
}

/// Sets an output twice: only the last one is kept.
struct TwiceReviewer;

impl WorkflowHandler for TwiceReviewer {
    fn name(&self) -> &str {
        "twice-reviewer"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.set_output(&verdict(false))?;
            ctx.set_output(&verdict(true))?;
            Ok(())
        })
    }
}

/// A child that never sets an output.
struct Silent;

impl WorkflowHandler for Silent {
    fn name(&self) -> &str {
        "silent"
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

impl TypedWorkflow for Silent {
    type Input = NoInput;
}

/// Sets its output, then sleeps five minutes. Counts its executions.
struct SleepyReviewer {
    executions: Arc<Mutex<u32>>,
}

impl WorkflowHandler for SleepyReviewer {
    fn name(&self) -> &str {
        "sleepy-reviewer"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            *self.executions.lock().expect("executions lock") += 1;
            ctx.set_output(&verdict(true))?;
            ctx.delay("pause", DelayConfig::from_secs(300)).await?;
            Ok(())
        })
    }
}

/// Sets a value that cannot serialize: only fine while planning.
struct Planner;

impl WorkflowHandler for Planner {
    fn name(&self) -> &str {
        "planner"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Non-string map keys: `serde_json` refuses them.
            let unserializable = HashMap::from([((1u8, 2u8), 3u8)]);
            ctx.set_output(&unserializable)?;
            Ok(())
        })
    }
}

/// Which child [`Caller`] runs, and how it reads its output.
#[derive(Clone, Copy)]
enum Call {
    /// [`Reviewer`], read as a [`Verdict`].
    Reviewer,
    /// [`Silent`], read as a [`Verdict`].
    Silent,
    /// [`Reviewer`], read as a number.
    WrongType,
    /// A failing [`Reviewer`] tolerated by `allow_failure`.
    AllowedFailure,
    /// [`Reviewer`], then a five-minute delay in the parent.
    ThenSleep,
}

/// A parent that runs a child and records the output it reads back.
struct Caller {
    name: &'static str,
    call: Call,
    seen: Seen,
}

impl WorkflowHandler for Caller {
    fn name(&self) -> &str {
        self.name
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let approve = ReviewInput {
                approved: true,
                fail: false,
            };
            match self.call {
                Call::Reviewer => {
                    let child = ctx.workflow(&Reviewer, approve).await?;
                    let read = child.output::<Verdict>();
                    // The parent forwards the verdict as its own output.
                    if let Ok(Some(verdict)) = &read {
                        ctx.set_output(verdict)?;
                    }
                    record(&self.seen, read);
                }
                Call::Silent => {
                    let child = ctx.workflow(&Silent, NoInput {}).await?;
                    record(&self.seen, child.output::<Verdict>());
                }
                Call::WrongType => {
                    let child = ctx.workflow(&Reviewer, approve).await?;
                    // A JSON object never deserializes into a number.
                    record(&self.seen, child.output::<u32>().map(|_| None));
                }
                Call::AllowedFailure => {
                    let outcome = ctx
                        .workflow_with(
                            &Reviewer,
                            ReviewInput {
                                approved: false,
                                fail: true,
                            },
                            WorkflowOptions::new().allow_failure(),
                        )
                        .await?;
                    let child = match outcome {
                        SubWorkflowOutcome::Completed(child) => child,
                        SubWorkflowOutcome::Conflict(_) => {
                            return Err(EngineError::InvalidWorkflow(
                                "no concurrency key was set".to_string(),
                            ));
                        }
                    };
                    record(&self.seen, child.output::<Verdict>());
                }
                Call::ThenSleep => {
                    let child = ctx.workflow(&Reviewer, approve).await?;
                    record(&self.seen, child.output::<Verdict>());
                    ctx.delay("pause", DelayConfig::from_secs(300)).await?;
                }
            }
            Ok(())
        })
    }
}

impl TypedWorkflow for Caller {
    type Input = NoInput;
}

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        "/tmp/ironflow-fixtures",
    ))
}

/// An engine with every handler of this file registered.
fn new_engine(store: &Arc<InMemoryStore>) -> (Arc<Engine>, Seen, Arc<Mutex<u32>>) {
    let dyn_store: Arc<dyn Store> = store.clone();
    let mut engine = Engine::new(dyn_store, provider());
    let seen = Seen::default();
    let executions = Arc::new(Mutex::new(0));

    engine.register(Reviewer).expect("register reviewer");
    engine.register(TwiceReviewer).expect("register twice");
    engine.register(Silent).expect("register silent");
    engine
        .register(SleepyReviewer {
            executions: executions.clone(),
        })
        .expect("register sleepy");
    engine.register(Planner).expect("register planner");
    for (name, call) in [
        ("call-reviewer", Call::Reviewer),
        ("call-silent", Call::Silent),
        ("call-wrong-type", Call::WrongType),
        ("call-allowed-failure", Call::AllowedFailure),
        ("call-then-sleep", Call::ThenSleep),
    ] {
        engine
            .register(Caller {
                name,
                call,
                seen: seen.clone(),
            })
            .expect("register caller");
    }
    (Arc::new(engine), seen, executions)
}

async fn load_run(store: &InMemoryStore, run_id: Uuid) -> Run {
    store
        .get_run(run_id)
        .await
        .expect("get run")
        .expect("run exists")
}

/// The single run of exactly `workflow`.
async fn run_of(store: &InMemoryStore, workflow: &str) -> Run {
    let filter = RunFilter {
        workflow_name: Some(workflow.to_string()),
        ..RunFilter::default()
    };
    let mut runs = store
        .list_runs(filter, 1, 50)
        .await
        .expect("list runs")
        .items;
    // The store filter matches on a substring: keep the exact name only.
    runs.retain(|r| r.workflow_name == workflow);
    assert_eq!(runs.len(), 1, "expected exactly one {workflow} run");
    runs.remove(0)
}

/// Poll the store until the run reaches `status`.
async fn wait_for_status(store: &InMemoryStore, run_id: Uuid, status: RunStatus) {
    timeout(TEST_TIMEOUT, async {
        loop {
            if load_run(store, run_id).await.status.state == status {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run never reached the expected status");
}

/// Move the wake-up of a sleeping run to the past and let the waker resume it.
async fn wake(engine: &Arc<Engine>, store: &InMemoryStore, run_id: Uuid) {
    store
        .update_run(
            run_id,
            RunUpdate {
                scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                ..RunUpdate::default()
            },
        )
        .await
        .expect("move the wake-up");
    let woken = RunWaker::new(engine.clone()).tick().await.expect("tick");
    assert_eq!(woken.iter().map(|r| r.id).collect::<Vec<_>>(), vec![run_id]);
}

fn reviewer_payload(approved: bool, fail: bool) -> Value {
    to_value(ReviewInput { approved, fail }).expect("serialize the input")
}

// -- A single run --

#[tokio::test]
async fn set_output_is_persisted_on_the_completed_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _, _) = new_engine(&store);

        let result = engine
            .run_handler(
                "reviewer",
                TriggerKind::Manual,
                reviewer_payload(true, false),
            )
            .await
            .expect("the run completes");

        assert_eq!(result.run.status.state, RunStatus::Completed);
        let expected = to_value(verdict(true)).expect("serialize");
        assert_eq!(result.run.output, Some(expected.clone()));
        assert_eq!(load_run(&store, result.run.id).await.output, Some(expected));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_last_call_wins() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _, _) = new_engine(&store);

        let result = engine
            .run_handler("twice-reviewer", TriggerKind::Manual, json!({}))
            .await
            .expect("the run completes");

        assert_eq!(
            load_run(&store, result.run.id).await.output,
            Some(to_value(verdict(true)).expect("serialize"))
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_survives_a_handler_error() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _, _) = new_engine(&store);

        let outcome = engine
            .run_handler(
                "reviewer",
                TriggerKind::Manual,
                reviewer_payload(false, true),
            )
            .await;
        assert!(outcome.is_err(), "the handler error fails the run");

        let run = run_of(&store, "reviewer").await;
        assert_eq!(run.status.state, RunStatus::Failed);
        assert!(run.error.is_some());
        assert_eq!(
            run.output,
            Some(to_value(verdict(false)).expect("serialize")),
            "the verdict set before the error is kept"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_is_not_written_while_sleeping_and_is_set_again_after_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _, executions) = new_engine(&store);

        let result = engine
            .run_handler("sleepy-reviewer", TriggerKind::Manual, json!({}))
            .await
            .expect("the run sleeps");
        assert_eq!(result.run.status.state, RunStatus::Sleeping);
        assert!(
            load_run(&store, result.run.id).await.output.is_none(),
            "a suspended execution writes no output"
        );

        wake(&engine, &store, result.run.id).await;
        wait_for_status(&store, result.run.id, RunStatus::Completed).await;

        assert_eq!(*executions.lock().expect("executions lock"), 2);
        assert_eq!(
            load_run(&store, result.run.id).await.output,
            Some(to_value(verdict(true)).expect("serialize")),
            "the replayed handler sets the output once more, and it replaces nothing else"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_is_a_noop_while_planning() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _, _) = new_engine(&store);

        let plan = engine
            .plan_handler("planner", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        assert!(
            plan.incomplete_reason.is_none(),
            "set_output did not fail the plan: {:?}",
            plan.incomplete_reason
        );
        assert!(!plan.truncated);
        assert!(
            store
                .list_runs(RunFilter::default(), 1, 10)
                .await
                .expect("list runs")
                .items
                .is_empty(),
            "planning creates no run"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_rejects_an_unserializable_value_and_fails_the_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _, _) = new_engine(&store);

        let err = engine
            .run_handler("planner", TriggerKind::Manual, json!({}))
            .await
            .expect_err("the value does not serialize");
        assert!(matches!(err, EngineError::Serialization(_)), "got {err:?}");

        let run = run_of(&store, "planner").await;
        assert_eq!(run.status.state, RunStatus::Failed);
        assert!(run.output.is_none());
    })
    .await
    .expect("test timed out");
}

// -- A parent reading its child --

#[tokio::test]
async fn set_output_child_output_is_readable_by_the_parent() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_outputs, _) = new_engine(&store);

        let result = engine
            .run_handler("call-reviewer", TriggerKind::Manual, json!({}))
            .await
            .expect("the chain completes");

        assert_eq!(result.run.status.state, RunStatus::Completed);
        assert_eq!(seen(&seen_outputs), vec![Ok(Some(verdict(true)))]);

        let expected = to_value(verdict(true)).expect("serialize");
        assert_eq!(
            run_of(&store, "reviewer").await.output,
            Some(expected.clone())
        );
        assert_eq!(
            load_run(&store, result.run.id).await.output,
            Some(expected),
            "the parent forwarded the verdict as its own output"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_child_output_is_none_when_the_child_sets_nothing() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_outputs, _) = new_engine(&store);

        let result = engine
            .run_handler("call-silent", TriggerKind::Manual, json!({}))
            .await
            .expect("the chain completes");

        assert_eq!(result.run.status.state, RunStatus::Completed);
        assert_eq!(seen(&seen_outputs), vec![Ok(None)]);
        assert!(run_of(&store, "silent").await.output.is_none());
        assert!(load_run(&store, result.run.id).await.output.is_none());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_child_output_of_the_wrong_type_returns_an_error() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_outputs, _) = new_engine(&store);

        engine
            .run_handler("call-wrong-type", TriggerKind::Manual, json!({}))
            .await
            .expect("the parent records the error and completes");

        assert_eq!(seen(&seen_outputs), vec![Err("serialization".to_string())]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_child_output_is_replayed_from_the_workflow_step_after_a_parent_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_outputs, _) = new_engine(&store);

        let parent = engine
            .run_handler("call-then-sleep", TriggerKind::Manual, json!({}))
            .await
            .expect("the parent sleeps")
            .run;
        assert_eq!(parent.status.state, RunStatus::Sleeping);

        // Rewrite the child output: a replay that read the child run back
        // would see this value instead of the recorded one.
        let child = run_of(&store, "reviewer").await;
        store
            .update_run(
                child.id,
                RunUpdate {
                    output: Some(json!({"approved": false, "score": 0})),
                    ..RunUpdate::default()
                },
            )
            .await
            .expect("rewrite the child output");

        wake(&engine, &store, parent.id).await;
        wait_for_status(&store, parent.id, RunStatus::Completed).await;

        assert_eq!(
            seen(&seen_outputs),
            vec![Ok(Some(verdict(true))), Ok(Some(verdict(true)))],
            "the resumed parent reads the output recorded in its workflow step"
        );
        assert_eq!(
            run_of(&store, "reviewer").await.id,
            child.id,
            "the replay creates no new child"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn set_output_allow_failure_child_keeps_its_output() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_outputs, _) = new_engine(&store);

        let result = engine
            .run_handler("call-allowed-failure", TriggerKind::Manual, json!({}))
            .await
            .expect("the tolerated failure completes the parent");

        assert_eq!(result.run.status.state, RunStatus::Warning);
        assert_eq!(seen(&seen_outputs), vec![Ok(Some(verdict(false)))]);

        let child = run_of(&store, "reviewer").await;
        assert_eq!(child.status.state, RunStatus::Failed);
        assert_eq!(
            child.output,
            Some(to_value(verdict(false)).expect("serialize"))
        );
    })
    .await
    .expect("test timed out");
}
