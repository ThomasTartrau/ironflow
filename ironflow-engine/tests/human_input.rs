//! Integration tests for typed human input steps (`ctx.human_input::<T>()`).
//!
//! Every test drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`]. An answer is written on the step through the public
//! store API, the way the API server does, and the run is resumed.
//!
//! Test names contain `human_input` so `cargo test -p ironflow-engine
//! human_input` selects the whole suite.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, from_value, json};
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::error::OperationError;
use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{
    Approvers, Assignee, HUMAN_INPUT_SCHEMA_KEY, HumanInputConfig, ShellConfig,
};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::escalation::{ApprovalEscalator, EscalationAction};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{WorkflowEvent, WorkflowEventBus, WorkflowInputRequiredEvent};
use ironflow_engine::plan::PlanOptions;
use ironflow_engine::testing::{HumanInputOutcome, MockShellOutput, TestEngine};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    RunFilter, RunStatus, RunUpdate, Step, StepKind, StepStatus, StepUpdate, TriggerKind,
};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch sockets or spawn processes.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`Clarify`].
const WORKFLOW: &str = "clarify";

/// The typed answer the handler asks for.
#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// An answer type that can be built from `{}`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
struct Options {
    verbose: bool,
}

/// What the handler does with a rejected input.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OnReject {
    Propagate,
    Continue,
}

/// What the handler saw, shared with the test.
type Seen = Arc<Mutex<Vec<String>>>;

/// Asks for [`Answers`], records them, then runs one more step.
///
/// Fails once with a transient error after the input when `fail_first_attempt`
/// is set, so the run is retried.
struct Clarify {
    config: HumanInputConfig,
    on_reject: OnReject,
    seen: Seen,
    fail_first_attempt: bool,
    attempts: AtomicU32,
}

impl Clarify {
    fn new(config: HumanInputConfig, on_reject: OnReject) -> (Self, Seen) {
        let seen = Seen::default();
        let handler = Self {
            config,
            on_reject,
            seen: seen.clone(),
            fail_first_attempt: false,
            attempts: AtomicU32::new(0),
        };
        (handler, seen)
    }
}

impl WorkflowHandler for Clarify {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            match ctx
                .human_input::<Answers>("clarify", self.config.clone())
                .await
            {
                Ok(answers) => {
                    self.seen.lock().expect("seen lock").extend(answers.answers);
                }
                Err(EngineError::HumanInputRejected { reason, .. })
                    if self.on_reject == OnReject::Continue =>
                {
                    self.seen
                        .lock()
                        .expect("seen lock")
                        .push(format!("rejected: {reason}"));
                }
                Err(err) => return Err(err),
            }

            if self.fail_first_attempt && self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(EngineError::Operation(OperationError::Http {
                    status: Some(502),
                    message: "bad gateway".to_string(),
                }));
            }

            ctx.shell("after", ShellConfig::new("echo after")).await?;
            Ok(())
        })
    }
}

/// Asks for [`Options`], which accept `{}`, then runs one more step.
struct ClarifyOptions;

impl WorkflowHandler for ClarifyOptions {
    fn name(&self) -> &str {
        "clarify-options"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let options: Options = ctx
                .human_input("options", HumanInputConfig::new("Pick the options"))
                .await?;
            let command = if options.verbose { "echo -v" } else { "echo" };
            ctx.shell("after", ShellConfig::new(command)).await?;
            Ok(())
        })
    }
}

/// Runs a step, asks for [`Answers`] and records them.
///
/// Registered under [`WORKFLOW`] so [`start`] drives it.
struct ClarifyWithBefore {
    seen: Seen,
}

impl WorkflowHandler for ClarifyWithBefore {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("before", ShellConfig::new("true")).await?;
            let answers: Answers = ctx
                .human_input("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            self.seen.lock().expect("seen lock").extend(answers.answers);
            Ok(())
        })
    }
}

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        "/tmp/ironflow-fixtures",
    ))
}

fn engine_with(store: Arc<InMemoryStore>, handler: impl WorkflowHandler + 'static) -> Engine {
    let store: Arc<dyn Store> = store;
    let mut engine = Engine::new(store, provider());
    engine.register(handler).expect("register handler");
    engine
}

/// Enqueue and execute a run the way the worker does, returning its id.
async fn start(engine: &Engine, store: &Arc<InMemoryStore>, max_retries: u32) -> Uuid {
    let run = engine
        .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), max_retries)
        .await
        .expect("enqueue");
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("mark running");
    engine
        .execute_handler_run(run.id)
        .await
        .expect("handler suspends on the input");
    run.id
}

/// The human input steps of a run, oldest first.
async fn input_steps(store: &Arc<InMemoryStore>, run_id: Uuid) -> Vec<Step> {
    store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .filter(|s| s.kind == StepKind::HumanInput)
        .collect()
}

/// The single human input step of a run.
async fn input_step(store: &Arc<InMemoryStore>, run_id: Uuid) -> Step {
    let mut steps = input_steps(store, run_id).await;
    assert_eq!(steps.len(), 1, "expected exactly one human input step");
    steps.remove(0)
}

/// Write an answer on the step and mark the run running, like the API does.
async fn answer(store: &Arc<InMemoryStore>, run_id: Uuid, step_id: Uuid, value: Value) {
    store
        .update_step(
            step_id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                output: Some(value),
                completed_at: Some(Utc::now()),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await
        .expect("store the answer");
    store
        .update_run_status(run_id, RunStatus::Running)
        .await
        .expect("mark running");
}

/// Reject the step and mark the run running, like the API does.
async fn reject(store: &Arc<InMemoryStore>, run_id: Uuid, step_id: Uuid, reason: &str) {
    store
        .update_step(
            step_id,
            StepUpdate {
                status: Some(StepStatus::Rejected),
                error: Some(reason.to_string()),
                completed_at: Some(Utc::now()),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await
        .expect("reject the input");
    store
        .update_run_status(run_id, RunStatus::Running)
        .await
        .expect("mark running");
}

fn seen(seen: &Seen) -> Vec<String> {
    seen.lock().expect("seen lock").clone()
}

#[tokio::test]
async fn human_input_suspends_the_run_and_persists_the_schema() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (handler, seen_answers) = Clarify::new(
            HumanInputConfig::new("Answer the questions"),
            OnReject::Propagate,
        );
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store, 0).await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);

        let step = input_step(&store, run_id).await;
        assert_eq!(step.status.state, StepStatus::AwaitingApproval);
        let input = step.input.expect("the step stores its input");
        assert_eq!(input["message"], json!("Answer the questions"));
        assert!(
            input[HUMAN_INPUT_SCHEMA_KEY]["properties"]["answers"].is_object(),
            "got {input}"
        );
        assert!(seen(&seen_answers).is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_resumes_with_the_typed_answer() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (handler, seen_answers) =
            Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Propagate);
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store, 0).await;
        let step = input_step(&store, run_id).await;
        answer(
            &store,
            run_id,
            step.id,
            json!({"answers": ["staging", "eu-west"]}),
        )
        .await;

        engine.resume_run(run_id).await.expect("resume");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
        assert_eq!(seen(&seen_answers), vec!["staging", "eu-west"]);
        assert_eq!(input_steps(&store, run_id).await.len(), 1);
    })
    .await
    .expect("test timed out");
}

/// A run requeued after its input was answered keeps `retry_count == 0`, and
/// a worker picks it up through `execute_handler_run`, not `resume_run`: the
/// step before the input must replay from cache and the answer must be found.
#[tokio::test]
async fn human_input_execute_handler_run_replays_answer_without_rerunning_prior_steps() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let seen_answers = Seen::default();
        let handler = ClarifyWithBefore {
            seen: seen_answers.clone(),
        };
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store, 0).await;
        let step = input_step(&store, run_id).await;
        answer(&store, run_id, step.id, json!({"answers": ["ok"]})).await;

        engine
            .execute_handler_run(run_id)
            .await
            .expect("resume via worker pickup");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
        assert_eq!(run.retry_count, 0);
        assert_eq!(seen(&seen_answers), vec!["ok"]);
        let before = store
            .list_steps(run_id)
            .await
            .unwrap()
            .iter()
            .filter(|s| s.name == "before")
            .count();
        assert_eq!(before, 1);
        assert_eq!(input_steps(&store, run_id).await.len(), 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_answer_that_does_not_match_the_type_fails_the_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (handler, _) = Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Propagate);
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store, 1).await;
        let step = input_step(&store, run_id).await;
        answer(&store, run_id, step.id, json!({"answers": 3})).await;

        let err = engine.resume_run(run_id).await.expect_err("type mismatch");
        assert!(matches!(err, EngineError::StepConfig(_)), "got {err:?}");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.retry_count, 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_rejected_step_reaches_the_handler() {
    timeout(TEST_TIMEOUT, async {
        // A handler that catches the rejection keeps going.
        let store = Arc::new(InMemoryStore::new());
        let (handler, seen_answers) =
            Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Continue);
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store, 0).await;
        let step = input_step(&store, run_id).await;
        reject(&store, run_id, step.id, "out of scope").await;

        engine.resume_run(run_id).await.expect("resume");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
        assert_eq!(seen(&seen_answers), vec!["rejected: out of scope"]);

        // A handler that propagates it fails the run, and the run is not retried.
        let store = Arc::new(InMemoryStore::new());
        let (handler, _) = Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Propagate);
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store, 2).await;
        let step = input_step(&store, run_id).await;
        reject(&store, run_id, step.id, "out of scope").await;

        let err = engine.resume_run(run_id).await.expect_err("rejected");
        match err {
            EngineError::HumanInputRejected {
                run_id: rejected_run,
                step_id,
                reason,
            } => {
                assert_eq!(rejected_run, run_id);
                assert_eq!(step_id, step.id);
                assert_eq!(reason, "out of scope");
            }
            other => panic!("expected HumanInputRejected, got {other:?}"),
        }

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.retry_count, 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_resumed_without_answer_suspends_again() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (handler, seen_answers) =
            Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Propagate);
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store, 0).await;
        let before = input_step(&store, run_id).await;

        store
            .update_run_status(run_id, RunStatus::Running)
            .await
            .unwrap();
        engine
            .resume_run(run_id)
            .await
            .expect("resume suspends again");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);

        let after = input_step(&store, run_id).await;
        assert_eq!(after.id, before.id, "no new step is created");
        assert_eq!(after.status.state, StepStatus::AwaitingApproval);
        assert!(seen(&seen_answers).is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_answer_is_carried_over_to_a_retry() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (mut handler, seen_answers) =
            Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Propagate);
        handler.fail_first_attempt = true;
        let engine = engine_with(store.clone(), handler);

        // Attempt 1 suspends, gets its answer, then fails past the input.
        let run_id = start(&engine, &store, 1).await;
        let step = input_step(&store, run_id).await;
        let value = json!({"answers": ["yes"]});
        answer(&store, run_id, step.id, value.clone()).await;
        assert!(engine.resume_run(run_id).await.is_err());

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Retrying);

        // Attempt 2 runs to completion without asking again.
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
        let picked = store.pick_next_pending(None).await.unwrap().unwrap();
        assert_eq!(picked.id, run_id);
        engine
            .execute_handler_run(run_id)
            .await
            .expect("retry runs past the input");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);

        let steps = input_steps(&store, run_id).await;
        let carried = steps
            .iter()
            .find(|s| s.attempt == 2)
            .expect("attempt 2 records the carried-over input");
        assert_eq!(carried.status.state, StepStatus::Completed);
        assert_eq!(carried.output, Some(value));
        assert_eq!(seen(&seen_answers), vec!["yes", "yes"]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_publishes_input_required_event() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (handler, _) = Clarify::new(
            HumanInputConfig::new("Answer the questions"),
            OnReject::Propagate,
        );
        let mut engine = engine_with(store.clone(), handler);
        let bus = WorkflowEventBus::new();
        engine.set_event_bus(bus.clone());

        let run = engine
            .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), 0)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        let mut rx = bus.subscribe(run.id);
        engine.execute_handler_run(run.id).await.unwrap();

        let mut events = Vec::new();
        while let Ok(Ok(event)) = timeout(Duration::from_millis(100), rx.recv()).await {
            events.push(event);
        }

        let inputs: Vec<&WorkflowInputRequiredEvent> = events
            .iter()
            .filter_map(|e| match e {
                WorkflowEvent::InputRequired(payload) => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(inputs.len(), 1, "expected one InputRequired event");

        let step = input_step(&store, run.id).await;
        let payload = inputs[0];
        assert_eq!(payload.run_id, run.id);
        assert_eq!(payload.step_id, step.id);
        assert_eq!(payload.step_name, "clarify");
        assert_eq!(payload.step_index, 0);
        assert_eq!(payload.message, "Answer the questions");
        assert!(payload.schema["properties"]["answers"].is_object());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_plan_mode_records_the_step_without_suspending() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (handler, _) = Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Propagate);
        let mut engine = engine_with(store.clone(), handler);
        engine.register(ClarifyOptions).expect("register");

        // `Answers` cannot be built from `{}`: the plan stops at the input.
        let plan = engine
            .plan_handler(WORKFLOW, json!({}), PlanOptions::default())
            .await
            .expect("plan built");
        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["clarify"]);
        assert_eq!(plan.steps[0].kind, StepKind::HumanInput);
        assert!(plan.truncated);
        let reason = plan.incomplete_reason.expect("a reason");
        assert!(
            reason.contains("human input 'clarify' has no answer while planning"),
            "got {reason}"
        );

        // `Options` accepts `{}`: the plan continues past the input.
        let plan = engine
            .plan_handler("clarify-options", json!({}), PlanOptions::default())
            .await
            .expect("plan built");
        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["options", "after"]);
        assert_eq!(plan.steps[0].kind, StepKind::HumanInput);
        assert!(!plan.truncated);
        assert!(plan.incomplete_reason.is_none());

        // Planning never touches the store.
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert!(runs.items.is_empty(), "planning must not create a run");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_test_engine_mock_provides_the_answer() {
    timeout(TEST_TIMEOUT, async {
        let (handler, seen_answers) =
            Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Propagate);
        let result = TestEngine::new()
            .with_handler(handler)
            .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("after")))
            .with_mock_human_input(|name, cfg| {
                assert_eq!(name, "clarify");
                assert_eq!(cfg.message(), "Answer?");
                HumanInputOutcome::Provided(json!({"answers": ["mocked"]}))
            })
            .run(json!({}))
            .await
            .expect("run");

        assert_eq!(result.status(), RunStatus::Completed);
        assert_eq!(seen(&seen_answers), vec!["mocked"]);
        let step = result.step("clarify");
        assert_eq!(step.kind(), &StepKind::HumanInput);
        assert_eq!(step.status(), StepStatus::Completed);
        assert_eq!(step.output(), &json!({"answers": ["mocked"]}));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_test_engine_mock_rejects() {
    timeout(TEST_TIMEOUT, async {
        let (handler, _) = Clarify::new(HumanInputConfig::new("Answer?"), OnReject::Propagate);
        let result = TestEngine::new()
            .with_handler(handler)
            .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("after")))
            .with_mock_human_input(|_name, _cfg| HumanInputOutcome::reject("not now"))
            .run(json!({}))
            .await
            .expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let step = result.step("clarify");
        assert_eq!(step.status(), StepStatus::Rejected);
        assert_eq!(step.error(), Some("not now"));
        assert!(result.try_step("after").is_none());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_deadline_and_assignee_are_armed() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = HumanInputConfig::new("Answer?")
            .with_deadline_secs(600)
            .assigned_to(Assignee::user("alice"))
            .requiring(Approvers::at_least(2).from_groups(["product"]));
        let (handler, _) = Clarify::new(config, OnReject::Propagate);
        let engine = engine_with(store.clone(), handler);

        let before = Utc::now();
        let run_id = start(&engine, &store, 0).await;
        let step = input_step(&store, run_id).await;

        let deadline = step.approval_deadline_at.expect("a deadline");
        assert!(
            deadline >= before + TimeDelta::seconds(599),
            "got {deadline}"
        );
        assert_eq!(step.approval_assignee, Some(Assignee::user("alice")));
        let requirement = step.approval_requirement.expect("a requirement");
        assert_eq!(requirement.required_approvers, 2);
        assert_eq!(requirement.approver_groups, vec!["product".to_string()]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn human_input_auto_approve_on_timeout_is_treated_as_a_rejection() {
    timeout(TEST_TIMEOUT, async {
        // `on_timeout` refuses AutoApprove; a stored config can still carry it.
        let config: HumanInputConfig = from_value(json!({
            "message": "Answer?",
            "deadline_secs": 1,
            "on_timeout": "auto_approve",
        }))
        .expect("deserialize");
        let store = Arc::new(InMemoryStore::new());
        let (handler, seen_answers) = Clarify::new(config, OnReject::Propagate);
        let engine = Arc::new(engine_with(store.clone(), handler));

        let run_id = start(&engine, &store, 0).await;
        let step = input_step(&store, run_id).await;
        store
            .update_step(
                step.id,
                StepUpdate {
                    approval_deadline_at: Some(Utc::now() - TimeDelta::seconds(1)),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("backdate the deadline");

        let records = ApprovalEscalator::new(engine.clone())
            .tick()
            .await
            .expect("tick");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, EscalationAction::Rejected);

        let step = store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::Failed);
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert!(seen(&seen_answers).is_empty());
    })
    .await
    .expect("test timed out");
}
