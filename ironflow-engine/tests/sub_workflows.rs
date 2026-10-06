//! Integration tests for typed sub-workflows.
//!
//! A child declares its input through [`TypedWorkflow`]; the parent calls
//! `ctx.workflow(&Child, ChildInput { .. })` and gets the child run id back as
//! a [`Uuid`]. Every test drives a real [`Engine`] over a real
//! [`InMemoryStore`] and real shell steps.
//!
//! A child that suspends (human input, signal wait, delay) suspends its whole
//! chain; resuming the child resumes the root run, which re-enters the same
//! child run. Test names contain `sub_workflow` so `cargo test -p
//! ironflow-engine sub_workflow` selects them.

use std::collections::HashMap;
use std::future::pending;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::spawn;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::{AgentProvider, LABEL_ROOT_RUN_ID};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{DelayConfig, HumanInputConfig, ShellConfig, WorkflowOptions};
use ironflow_engine::context::{PARENT_RUN_ID_LABEL, WorkflowContext};
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::executor::SubWorkflowOutcome;
use ironflow_engine::guard::WorkflowGuardConfig;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::plan::{ConditionResult, PlanOptions};
use ironflow_engine::signal::Signal;
use ironflow_engine::wake::RunWaker;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    LeaseRequest, NewRun, Run, RunFilter, RunStatus, RunUpdate, Step, StepKind, StepStatus,
    StepUpdate, TriggerKind,
};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that spawn processes.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Input of [`Greeter`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct GreetInput {
    name: String,
    shout: bool,
}

/// A child whose input is typed.
struct Greeter;

impl WorkflowHandler for Greeter {
    fn name(&self) -> &str {
        "greeter"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            if ctx.when("shouting", |i: &GreetInput| i.shout).await? {
                ctx.shell("greet-loud", ShellConfig::new("echo HELLO"))
                    .await?;
            } else {
                ctx.shell("greet", ShellConfig::new("echo hello")).await?;
            }
            Ok(())
        })
    }
}

impl TypedWorkflow for Greeter {
    type Input = GreetInput;
}

/// Calls [`Greeter`] and remembers the child run id it got back.
struct Host {
    child_run_id: Arc<Mutex<Option<Uuid>>>,
}

impl WorkflowHandler for Host {
    fn name(&self) -> &str {
        "host"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let child = ctx
                .workflow(
                    &Greeter,
                    GreetInput {
                        name: "Élodie".to_string(),
                        shout: true,
                    },
                )
                .await?;
            *self.child_run_id.lock().expect("lock") = Some(child.run_id());
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

fn engine(store: Arc<InMemoryStore>, child_run_id: Arc<Mutex<Option<Uuid>>>) -> Engine {
    let store: Arc<dyn Store> = store;
    let mut engine = Engine::new(store, provider());
    engine.register(Greeter).expect("register child");
    engine
        .register(Host { child_run_id })
        .expect("register parent");
    engine
}

#[tokio::test]
async fn a_typed_sub_workflow_runs_with_its_input_and_reports_its_run_id() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let seen = Arc::new(Mutex::new(None));
        let engine = engine(store.clone(), seen.clone());

        let result = engine
            .run_handler("host", TriggerKind::Manual, json!({}))
            .await
            .expect("parent completes");
        assert_eq!(result.run.status.state, RunStatus::Completed);

        let child = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs")
            .items
            .into_iter()
            .find(|r| r.workflow_name == "greeter")
            .expect("child run exists");

        assert_eq!(*seen.lock().expect("lock"), Some(child.id));
        assert_eq!(child.payload, json!({"name": "Élodie", "shout": true}));

        let child_steps = store.list_steps(child.id).await.expect("child steps");
        assert_eq!(child_steps.len(), 1);
        assert_eq!(child_steps[0].name, "greet-loud");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_planned_sub_workflow_is_expanded_with_its_typed_input_and_a_nil_run_id() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let seen = Arc::new(Mutex::new(None));
        let engine = engine(store.clone(), seen.clone());

        let plan = engine
            .plan_handler("host", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        assert!(
            !plan.truncated,
            "plan stopped: {:?}",
            plan.incomplete_reason
        );
        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["greeter", "greet-loud"]);
        assert_eq!(plan.steps[0].kind, StepKind::Workflow);
        match plan.steps[1].condition.as_ref().expect("a condition") {
            ConditionResult::Evaluated { expression, value } => {
                assert_eq!(expression, "shouting");
                assert!(*value);
            }
            other => panic!("expected an evaluated condition, got {other:?}"),
        }

        assert_eq!(*seen.lock().expect("lock"), Some(Uuid::nil()));
        assert!(
            store
                .list_runs(RunFilter::default(), 1, 10)
                .await
                .expect("list runs")
                .items
                .is_empty(),
            "planning must not create runs"
        );
    })
    .await
    .expect("test timed out");
}

// -- Suspension of a child run --

/// Workflow name of [`Parent`].
const PARENT: &str = "parent";

/// Workflow name of [`Grandparent`].
const GRANDPARENT: &str = "grandparent";

/// Key [`Waiter`] waits on.
const SIGNAL_KEY: &str = "release-42";

/// Payload of a workflow that takes nothing.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoInput {}

/// The answer [`Asker`] waits for.
#[derive(Debug, Deserialize, JsonSchema)]
struct NameAnswer {
    name: String,
}

/// The signal [`Waiter`] waits for.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Deployed {
    version: String,
}

impl Signal for Deployed {
    const NAME: &'static str = "test.deployed";
}

/// What the suspending children saw once resumed, shared with the test.
type Seen = Arc<Mutex<Vec<String>>>;

fn seen(seen: &Seen) -> Vec<String> {
    seen.lock().expect("seen lock").clone()
}

/// A child that asks a human for a name.
struct Asker {
    seen: Seen,
}

impl WorkflowHandler for Asker {
    fn name(&self) -> &str {
        "asker"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let answer: NameAnswer = ctx
                .human_input("ask-name", HumanInputConfig::new("Who ships it?"))
                .await?;
            self.seen.lock().expect("seen lock").push(answer.name);
            ctx.shell("after-input", ShellConfig::new("echo answered"))
                .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Asker {
    type Input = NoInput;
}

/// A child that waits for [`Deployed`] on [`SIGNAL_KEY`].
struct Waiter {
    seen: Seen,
}

impl WorkflowHandler for Waiter {
    fn name(&self) -> &str {
        "waiter"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let deployed = ctx
                .wait_for_signal::<Deployed>("wait-deploy", SIGNAL_KEY, Duration::from_secs(3600))
                .await?;
            let seen_value = match deployed {
                Some(deployed) => deployed.version,
                None => "timed out".to_string(),
            };
            self.seen.lock().expect("seen lock").push(seen_value);
            Ok(())
        })
    }
}

impl TypedWorkflow for Waiter {
    type Input = NoInput;
}

/// A child that pauses for five minutes.
struct Sleeper {
    seen: Seen,
}

impl WorkflowHandler for Sleeper {
    fn name(&self) -> &str {
        "sleeper"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.delay("pause", DelayConfig::from_secs(300)).await?;
            self.seen
                .lock()
                .expect("seen lock")
                .push("slept".to_string());
            Ok(())
        })
    }
}

impl TypedWorkflow for Sleeper {
    type Input = NoInput;
}

/// What the suspending child of [`Parent`] waits on.
#[derive(Clone, Copy)]
enum Suspends {
    HumanInput,
    Signal,
    Delay,
}

/// Runs [`Greeter`] to completion, then a child that suspends, then a step.
struct Parent {
    suspends: Suspends,
    seen: Seen,
}

impl WorkflowHandler for Parent {
    fn name(&self) -> &str {
        PARENT
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(
                &Greeter,
                GreetInput {
                    name: "sibling".to_string(),
                    shout: false,
                },
            )
            .await?;
            let seen = self.seen.clone();
            match self.suspends {
                Suspends::HumanInput => ctx.workflow(&Asker { seen }, NoInput {}).await?,
                Suspends::Signal => ctx.workflow(&Waiter { seen }, NoInput {}).await?,
                Suspends::Delay => ctx.workflow(&Sleeper { seen }, NoInput {}).await?,
            };
            ctx.shell("after-child", ShellConfig::new("echo done"))
                .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Parent {
    type Input = NoInput;
}

/// Runs [`Parent`] as a sub-workflow: the suspending run is a grandchild.
struct Grandparent {
    suspends: Suspends,
    seen: Seen,
}

impl WorkflowHandler for Grandparent {
    fn name(&self) -> &str {
        GRANDPARENT
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let parent = Parent {
                suspends: self.suspends,
                seen: self.seen.clone(),
            };
            ctx.workflow(&parent, NoInput {}).await?;
            Ok(())
        })
    }
}

fn new_engine(store: &Arc<InMemoryStore>) -> Engine {
    let store: Arc<dyn Store> = store.clone();
    Engine::new(store, provider())
}

/// Register the whole chain on `engine`, `Parent` suspending on `suspends`.
fn build_chain(mut engine: Engine, suspends: Suspends) -> (Arc<Engine>, Seen) {
    let seen = Seen::default();
    engine.register(Greeter).expect("register greeter");
    engine
        .register(Asker { seen: seen.clone() })
        .expect("register asker");
    engine
        .register(Waiter { seen: seen.clone() })
        .expect("register waiter");
    engine
        .register(Sleeper { seen: seen.clone() })
        .expect("register sleeper");
    engine
        .register(Parent {
            suspends,
            seen: seen.clone(),
        })
        .expect("register parent");
    engine
        .register(Grandparent {
            suspends,
            seen: seen.clone(),
        })
        .expect("register grandparent");
    (Arc::new(engine), seen)
}

/// Run `root` until its chain suspends, returning the root run.
async fn start_chain(engine: &Engine, root: &str) -> Run {
    engine
        .run_handler(root, TriggerKind::Manual, json!({}))
        .await
        .expect("the chain suspends")
        .run
}

async fn load_run(store: &InMemoryStore, run_id: Uuid) -> Run {
    store
        .get_run(run_id)
        .await
        .expect("get run")
        .expect("run exists")
}

/// The single run of `workflow`.
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
    // The store filter matches on a substring (`parent` also matches
    // `grandparent`): keep the exact name only.
    runs.retain(|r| r.workflow_name == workflow);
    assert_eq!(runs.len(), 1, "expected exactly one {workflow} run");
    runs.remove(0)
}

/// The `Workflow` step of `run_id` that calls `child`.
async fn workflow_step(store: &InMemoryStore, run_id: Uuid, child: &str) -> Step {
    store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .find(|s| s.kind == StepKind::Workflow && s.name == child)
        .expect("the workflow step exists")
}

/// The open human input step of `run_id`.
async fn open_input_step(store: &InMemoryStore, run_id: Uuid) -> Step {
    let step = store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .find(|s| s.kind == StepKind::HumanInput)
        .expect("a human input step");
    assert_eq!(step.status.state, StepStatus::AwaitingApproval);
    step
}

/// Answer the human input of `run_id` and mark the run running, like the API does.
async fn answer(store: &InMemoryStore, run_id: Uuid, name: &str) {
    let step = open_input_step(store, run_id).await;
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                output: Some(json!({ "name": name })),
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

/// Reject the human input of `run_id` and mark the run running, like the API does.
async fn reject(store: &InMemoryStore, run_id: Uuid, reason: &str) {
    let step = open_input_step(store, run_id).await;
    store
        .update_step(
            step.id,
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

#[tokio::test]
async fn sub_workflow_human_input_suspends_the_parent_and_resumes_through_the_root() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_names) = build_chain(new_engine(&store), Suspends::HumanInput);

        let parent = start_chain(&engine, PARENT).await;
        assert_eq!(parent.status.state, RunStatus::AwaitingApproval);
        assert!(parent.scheduled_at.is_none());

        let child = run_of(&store, "asker").await;
        assert_eq!(child.status.state, RunStatus::AwaitingApproval);
        assert_eq!(child.trigger, TriggerKind::Workflow);

        let step = workflow_step(&store, parent.id, "asker").await;
        assert_eq!(
            step.status.state,
            StepStatus::Running,
            "the step stays open while its child is suspended"
        );
        assert_eq!(step.output, Some(json!({ "child_run_id": child.id })));
        assert!(seen(&seen_names).is_empty());

        answer(&store, child.id, "Ada").await;
        let result = engine
            .resume_run(child.id)
            .await
            .expect("the chain resumes");

        assert_eq!(result.run.id, parent.id, "the root run is the one resumed");
        assert_eq!(result.run.status.state, RunStatus::Completed);
        assert_eq!(
            run_of(&store, "asker").await.id,
            child.id,
            "the same child run is re-entered, never a new one"
        );
        assert_eq!(
            load_run(&store, child.id).await.status.state,
            RunStatus::Completed
        );

        let step = workflow_step(&store, parent.id, "asker").await;
        assert_eq!(step.status.state, StepStatus::Completed);
        let output: Value = step.output.expect("the step has an output");
        assert_eq!(output["run_id"], json!(child.id));
        assert_eq!(output["status"], json!("completed"));
        assert_eq!(seen(&seen_names), vec!["Ada".to_string()]);

        let parent_steps = store.list_steps(parent.id).await.expect("list steps");
        let names: Vec<&str> = parent_steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["greeter", "asker", "after-child"]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_completed_sibling_is_not_run_again_on_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _seen) = build_chain(new_engine(&store), Suspends::HumanInput);

        let parent = start_chain(&engine, PARENT).await;
        let sibling = run_of(&store, "greeter").await;
        assert_eq!(sibling.status.state, RunStatus::Completed);

        let child = run_of(&store, "asker").await;
        answer(&store, child.id, "Ada").await;
        engine
            .resume_run(child.id)
            .await
            .expect("the chain resumes");

        assert_eq!(
            run_of(&store, "greeter").await.id,
            sibling.id,
            "a completed child is replayed, never run again"
        );
        let sibling_steps = store.list_steps(sibling.id).await.expect("list steps");
        assert_eq!(sibling_steps.len(), 1);
        let step = workflow_step(&store, parent.id, "greeter").await;
        assert_eq!(step.status.state, StepStatus::Completed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_suspended_child_is_listed_by_its_chain_labels() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _seen) = build_chain(new_engine(&store), Suspends::HumanInput);

        let parent = start_chain(&engine, PARENT).await;

        let waiting = store
            .list_runs(
                RunFilter {
                    labels: Some(HashMap::from([(
                        PARENT_RUN_ID_LABEL.to_string(),
                        parent.id.to_string(),
                    )])),
                    status: Some(RunStatus::AwaitingApproval),
                    ..RunFilter::default()
                },
                1,
                50,
            )
            .await
            .expect("list runs")
            .items;
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].workflow_name, "asker");

        let chain = store
            .list_runs(
                RunFilter {
                    labels: Some(HashMap::from([(
                        LABEL_ROOT_RUN_ID.to_string(),
                        parent.id.to_string(),
                    )])),
                    ..RunFilter::default()
                },
                1,
                50,
            )
            .await
            .expect("list runs")
            .items;
        let mut names: Vec<&str> = chain.iter().map(|r| r.workflow_name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["asker", "greeter"]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_child_executed_by_a_worker_resumes_its_root() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_names) = build_chain(new_engine(&store), Suspends::HumanInput);

        let parent = start_chain(&engine, PARENT).await;
        let child = run_of(&store, "asker").await;

        // A worker picks the requeued child and executes it: the child must
        // not run on its own, outside of its parent.
        answer(&store, child.id, "Grace").await;
        let result = engine
            .execute_handler_run(child.id)
            .await
            .expect("the chain resumes");

        assert_eq!(result.run.id, parent.id);
        assert_eq!(result.run.status.state, RunStatus::Completed);
        assert_eq!(
            load_run(&store, child.id).await.status.state,
            RunStatus::Completed
        );
        assert_eq!(seen(&seen_names), vec!["Grace".to_string()]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_signal_wait_suspends_the_chain_until_the_signal_arrives() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_versions) = build_chain(new_engine(&store), Suspends::Signal);

        let parent = start_chain(&engine, PARENT).await;
        assert_eq!(parent.status.state, RunStatus::Sleeping);
        assert!(
            parent.scheduled_at.is_none(),
            "only the waiting child owns the wake-up"
        );

        let child = run_of(&store, "waiter").await;
        assert_eq!(child.status.state, RunStatus::Sleeping);
        assert!(child.scheduled_at.is_some(), "the child arms the deadline");

        let delivery = engine
            .send_signal(
                &Deployed {
                    version: "1.2.3".to_string(),
                },
                SIGNAL_KEY,
                None,
            )
            .await
            .expect("deliver");
        assert_eq!(delivery.resumed.len(), 1);
        assert_eq!(delivery.resumed[0].run_id, child.id);

        wait_for_status(&store, parent.id, RunStatus::Completed).await;
        assert_eq!(
            load_run(&store, child.id).await.status.state,
            RunStatus::Completed
        );
        assert_eq!(seen(&seen_versions), vec!["1.2.3".to_string()]);
        assert_eq!(run_of(&store, "waiter").await.id, child.id);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_delay_wakes_the_chain_through_the_waker() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_wakes) = build_chain(new_engine(&store), Suspends::Delay);

        let parent = start_chain(&engine, PARENT).await;
        assert_eq!(parent.status.state, RunStatus::Sleeping);
        assert!(parent.scheduled_at.is_none());

        let child = run_of(&store, "sleeper").await;
        assert_eq!(child.status.state, RunStatus::Sleeping);
        assert!(child.scheduled_at.is_some());

        // Move the wake-up to the past: the waker only claims a due run.
        store
            .update_run(
                child.id,
                RunUpdate {
                    scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                    ..RunUpdate::default()
                },
            )
            .await
            .expect("move the wake-up");

        let woken = RunWaker::new(engine.clone()).tick().await.expect("tick");
        assert_eq!(
            woken.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![child.id],
            "the parent is never woken on its own"
        );

        wait_for_status(&store, parent.id, RunStatus::Completed).await;
        assert_eq!(
            load_run(&store, child.id).await.status.state,
            RunStatus::Completed
        );
        assert_eq!(seen(&seen_wakes), vec!["slept".to_string()]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_rejected_human_input_fails_the_child_and_the_parent() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_names) = build_chain(new_engine(&store), Suspends::HumanInput);

        let parent = start_chain(&engine, PARENT).await;
        let child = run_of(&store, "asker").await;

        reject(&store, child.id, "nobody").await;
        let err = engine
            .resume_run(child.id)
            .await
            .expect_err("a rejected input fails the chain");
        assert!(
            matches!(err, EngineError::HumanInputRejected { .. }),
            "got {err}"
        );

        assert_eq!(
            load_run(&store, child.id).await.status.state,
            RunStatus::Failed
        );
        assert_eq!(
            load_run(&store, parent.id).await.status.state,
            RunStatus::Failed
        );
        let step = workflow_step(&store, parent.id, "asker").await;
        assert_eq!(step.status.state, StepStatus::Failed);
        assert!(seen(&seen_names).is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_child_of_a_cancelled_root_cannot_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_names) = build_chain(new_engine(&store), Suspends::HumanInput);

        let parent = start_chain(&engine, PARENT).await;
        let child = run_of(&store, "asker").await;
        store
            .update_run_status(parent.id, RunStatus::Cancelled)
            .await
            .expect("cancel the root");

        answer(&store, child.id, "Ada").await;
        let err = engine
            .resume_run(child.id)
            .await
            .expect_err("a cancelled root cannot take its child back");
        assert!(matches!(err, EngineError::InvalidWorkflow(_)), "got {err}");

        assert_eq!(
            load_run(&store, child.id).await.status.state,
            RunStatus::Failed
        );
        assert_eq!(
            load_run(&store, parent.id).await.status.state,
            RunStatus::Cancelled
        );
        assert!(seen(&seen_names).is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_nested_grandchild_suspends_and_resumes_the_whole_chain() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_names) = build_chain(new_engine(&store), Suspends::HumanInput);

        let root = start_chain(&engine, GRANDPARENT).await;
        assert_eq!(root.status.state, RunStatus::AwaitingApproval);

        let parent = run_of(&store, PARENT).await;
        assert_eq!(parent.status.state, RunStatus::AwaitingApproval);
        assert!(
            parent.scheduled_at.is_none(),
            "an intermediate run never owns a wake-up"
        );
        assert_eq!(
            parent.labels.get(PARENT_RUN_ID_LABEL),
            Some(&root.id.to_string())
        );

        let grandchild = run_of(&store, "asker").await;
        assert_eq!(grandchild.status.state, RunStatus::AwaitingApproval);
        assert_eq!(
            grandchild.labels.get(PARENT_RUN_ID_LABEL),
            Some(&parent.id.to_string())
        );
        assert_eq!(
            grandchild.labels.get(LABEL_ROOT_RUN_ID),
            Some(&root.id.to_string())
        );

        answer(&store, grandchild.id, "Ada").await;
        let result = engine
            .resume_run(grandchild.id)
            .await
            .expect("the chain resumes");

        assert_eq!(result.run.id, root.id);
        assert_eq!(result.run.status.state, RunStatus::Completed);
        for run_id in [parent.id, grandchild.id] {
            assert_eq!(
                load_run(&store, run_id).await.status.state,
                RunStatus::Completed
            );
        }
        assert_eq!(run_of(&store, PARENT).await.id, parent.id);
        assert_eq!(run_of(&store, "asker").await.id, grandchild.id);
        assert_eq!(
            run_of(&store, "greeter").await.status.state,
            RunStatus::Completed
        );
        assert_eq!(seen(&seen_names), vec!["Ada".to_string()]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_resume_stays_within_the_guard_fan_out() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        // Room for exactly the two children of `Parent`: the resumed
        // execution, which replays the sibling and re-enters the suspended
        // child, must stay within it.
        let engine =
            new_engine(&store).with_guard_config(WorkflowGuardConfig::new().with_max_fan_out(2));
        let (engine, seen_names) = build_chain(engine, Suspends::HumanInput);

        start_chain(&engine, PARENT).await;
        let child = run_of(&store, "asker").await;

        answer(&store, child.id, "Ada").await;
        let result = engine
            .resume_run(child.id)
            .await
            .expect("the guard lets the chain resume");

        assert_eq!(result.run.status.state, RunStatus::Completed);
        assert_eq!(seen(&seen_names), vec!["Ada".to_string()]);
    })
    .await
    .expect("test timed out");
}

/// Concurrency key [`Dispatcher`] puts on its [`Greeter`] child.
const ISSUE_KEY: &str = "issue:12";

/// What [`Dispatcher`] read from `workflow_with`, once per execution.
type Outcomes = Arc<Mutex<Vec<String>>>;

/// Calls [`Greeter`] under [`ISSUE_KEY`], then waits for a human, so a test
/// can resume it and watch the sub-workflow step replay.
struct Dispatcher {
    outcomes: Outcomes,
}

impl WorkflowHandler for Dispatcher {
    fn name(&self) -> &str {
        "dispatcher"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input = GreetInput {
                name: "Ada".to_string(),
                shout: false,
            };
            let options = WorkflowOptions::new().concurrency_key(ISSUE_KEY);
            let seen = match ctx.workflow_with(&Greeter, input, options).await? {
                SubWorkflowOutcome::Completed(child) => format!("completed:{}", child.run_id()),
                SubWorkflowOutcome::Conflict(c) => format!("conflict:{}:{}", c.key(), c.run_id()),
            };
            self.outcomes.lock().expect("outcomes lock").push(seen);

            let _confirmed: NameAnswer = ctx
                .human_input("confirm", HumanInputConfig::new("Who confirms?"))
                .await?;
            ctx.shell("after-confirm", ShellConfig::new("echo confirmed"))
                .await?;
            Ok(())
        })
    }
}

fn dispatcher_engine(store: &Arc<InMemoryStore>) -> (Engine, Outcomes) {
    let outcomes = Outcomes::default();
    let mut engine = new_engine(store);
    engine.register(Greeter).expect("register greeter");
    engine
        .register(Dispatcher {
            outcomes: outcomes.clone(),
        })
        .expect("register dispatcher");
    (engine, outcomes)
}

fn outcomes(outcomes: &Outcomes) -> Vec<String> {
    outcomes.lock().expect("outcomes lock").clone()
}

/// Every run of `workflow`, matched on the exact name.
async fn runs_of(store: &InMemoryStore, workflow: &str) -> Vec<Run> {
    let filter = RunFilter {
        workflow_name: Some(workflow.to_string()),
        ..RunFilter::default()
    };
    let mut runs = store
        .list_runs(filter, 1, 50)
        .await
        .expect("list runs")
        .items;
    runs.retain(|r| r.workflow_name == workflow);
    runs
}

/// A pending run holding [`ISSUE_KEY`], standing for a fix already under way.
async fn create_blocker(store: &InMemoryStore) -> Run {
    store
        .create_run(NewRun {
            workflow_name: "blocker".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            concurrency_key: Some(ISSUE_KEY.to_string()),
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("create the blocker")
        .into_run()
}

#[tokio::test]
async fn sub_workflow_concurrency_conflict_completes_the_step_without_a_child() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_outcomes) = dispatcher_engine(&store);
        let blocker = create_blocker(&store).await;

        let dispatcher = engine
            .run_handler("dispatcher", TriggerKind::Manual, json!({}))
            .await
            .expect("the dispatcher suspends on its human input")
            .run;

        assert_eq!(
            dispatcher.status.state,
            RunStatus::AwaitingApproval,
            "a conflict does not fail the parent"
        );
        assert!(
            runs_of(&store, "greeter").await.is_empty(),
            "no child run is created on a conflict"
        );
        assert_eq!(
            outcomes(&seen_outcomes),
            vec![format!("conflict:{ISSUE_KEY}:{}", blocker.id)]
        );

        let step = workflow_step(&store, dispatcher.id, "greeter").await;
        assert_eq!(step.status.state, StepStatus::Completed);
        assert_eq!(
            step.output,
            Some(json!({
                "concurrency_conflict": { "key": ISSUE_KEY, "run_id": blocker.id }
            }))
        );
        assert_eq!(step.duration_ms, 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_concurrency_conflict_is_replayed_without_a_new_child() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_outcomes) = dispatcher_engine(&store);
        let blocker = create_blocker(&store).await;

        let dispatcher = engine
            .run_handler("dispatcher", TriggerKind::Manual, json!({}))
            .await
            .expect("the dispatcher suspends on its human input")
            .run;
        assert_eq!(dispatcher.status.state, RunStatus::AwaitingApproval);

        // The key is free again before the resume: the replay must still
        // serve the recorded conflict instead of starting the child.
        store
            .update_run_status(blocker.id, RunStatus::Cancelled)
            .await
            .expect("cancel the blocker");
        answer(&store, dispatcher.id, "Grace").await;

        let result = engine
            .resume_run(dispatcher.id)
            .await
            .expect("the dispatcher resumes");

        assert_eq!(result.run.status.state, RunStatus::Completed);
        assert!(
            runs_of(&store, "greeter").await.is_empty(),
            "the replay does not create a child"
        );
        let conflict = format!("conflict:{ISSUE_KEY}:{}", blocker.id);
        assert_eq!(outcomes(&seen_outcomes), vec![conflict.clone(), conflict]);

        let steps = store.list_steps(dispatcher.id).await.expect("list steps");
        let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["greeter", "confirm", "after-confirm"]);
        assert_eq!(
            steps[0].output,
            Some(json!({
                "concurrency_conflict": { "key": ISSUE_KEY, "run_id": blocker.id }
            })),
            "the recorded conflict is left as it was"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_with_concurrency_key_creates_a_child_holding_the_key() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_outcomes) = dispatcher_engine(&store);

        let dispatcher = engine
            .run_handler("dispatcher", TriggerKind::Manual, json!({}))
            .await
            .expect("the dispatcher suspends on its human input")
            .run;
        assert_eq!(dispatcher.status.state, RunStatus::AwaitingApproval);

        let child = run_of(&store, "greeter").await;
        assert_eq!(child.status.state, RunStatus::Completed);
        assert_eq!(child.concurrency_key.as_deref(), Some(ISSUE_KEY));
        assert_eq!(
            outcomes(&seen_outcomes),
            vec![format!("completed:{}", child.id)]
        );

        let step = workflow_step(&store, dispatcher.id, "greeter").await;
        assert_eq!(step.status.state, StepStatus::Completed);
        assert_eq!(
            step.input.expect("the step records its config")["concurrency_key"],
            json!(ISSUE_KEY)
        );

        // The child is terminal: the key is free for the next run.
        let next = create_blocker(&store).await;
        assert_eq!(next.concurrency_key.as_deref(), Some(ISSUE_KEY));
    })
    .await
    .expect("test timed out");
}

// -- allow_failure on a sub-workflow step --

/// A child whose handler always fails.
struct Failer;

impl WorkflowHandler for Failer {
    fn name(&self) -> &str {
        "failer"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("boom", ShellConfig::new("exit 1")).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Failer {
    type Input = NoInput;
}

/// Tolerates a failing child, then runs a child that suspends, then a step.
struct Tolerant {
    seen: Seen,
}

impl WorkflowHandler for Tolerant {
    fn name(&self) -> &str {
        "tolerant"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let failed = match ctx
                .workflow_with(&Failer, NoInput {}, WorkflowOptions::new().allow_failure())
                .await?
            {
                SubWorkflowOutcome::Completed(output) => output,
                SubWorkflowOutcome::Conflict(_) => {
                    return Err(EngineError::InvalidWorkflow(
                        "no concurrency key was set".to_string(),
                    ));
                }
            };
            if failed.status() != RunStatus::Failed || failed.error().is_none() {
                return Err(EngineError::InvalidWorkflow(format!(
                    "the failed child was not reported: {:?} {:?}",
                    failed.status(),
                    failed.error()
                )));
            }
            ctx.workflow(
                &Asker {
                    seen: self.seen.clone(),
                },
                NoInput {},
            )
            .await?;
            ctx.shell("after-child", ShellConfig::new("echo done"))
                .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Tolerant {
    type Input = NoInput;
}

/// Calls a failing child without `allow_failure`.
struct Strict;

impl WorkflowHandler for Strict {
    fn name(&self) -> &str {
        "strict"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&Failer, NoInput {}).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Strict {
    type Input = NoInput;
}

/// Calls a suspending child with `allow_failure`.
struct TolerantAsker {
    seen: Seen,
}

impl WorkflowHandler for TolerantAsker {
    fn name(&self) -> &str {
        "tolerant-asker"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow_with(
                &Asker {
                    seen: self.seen.clone(),
                },
                NoInput {},
                WorkflowOptions::new().allow_failure(),
            )
            .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for TolerantAsker {
    type Input = NoInput;
}

/// An engine with the children and parents of the `allow_failure` tests.
fn allow_failure_engine(store: &Arc<InMemoryStore>) -> (Arc<Engine>, Seen) {
    let seen = Seen::default();
    let mut engine = new_engine(store);
    engine.register(Failer).expect("register failer");
    engine
        .register(Asker { seen: seen.clone() })
        .expect("register asker");
    engine
        .register(Tolerant { seen: seen.clone() })
        .expect("register tolerant");
    engine.register(Strict).expect("register strict");
    engine
        .register(TolerantAsker { seen: seen.clone() })
        .expect("register tolerant asker");
    (Arc::new(engine), seen)
}

/// How many runs of exactly `workflow` the store holds.
async fn count_runs(store: &InMemoryStore, workflow: &str) -> usize {
    let filter = RunFilter {
        workflow_name: Some(workflow.to_string()),
        ..RunFilter::default()
    };
    store
        .list_runs(filter, 1, 50)
        .await
        .expect("list runs")
        .items
        .iter()
        .filter(|r| r.workflow_name == workflow)
        .count()
}

#[tokio::test]
async fn sub_workflow_allow_failure_tolerates_a_failed_child_then_resumes_without_a_new_child() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_names) = allow_failure_engine(&store);

        let parent = start_chain(&engine, "tolerant").await;
        assert_eq!(parent.status.state, RunStatus::AwaitingApproval);

        let failer_step = workflow_step(&store, parent.id, "failer").await;
        assert_eq!(failer_step.status.state, StepStatus::Completed);
        assert_eq!(count_runs(&store, "failer").await, 1);

        let asker = run_of(&store, "asker").await;
        answer(&store, asker.id, "Ada").await;
        let result = engine
            .resume_run(asker.id)
            .await
            .expect("the chain resumes");

        assert_eq!(result.run.id, parent.id);
        assert_eq!(result.run.status.state, RunStatus::Warning);
        assert_eq!(
            count_runs(&store, "failer").await,
            1,
            "the resume replays the step and creates no new child"
        );

        let replayed = workflow_step(&store, parent.id, "failer").await;
        assert_eq!(replayed.id, failer_step.id);
        assert_eq!(replayed.status.state, StepStatus::Completed);

        let failer = run_of(&store, "failer").await;
        assert_eq!(failer.status.state, RunStatus::Failed);
        assert!(failer.error.is_some(), "the child error is recorded");

        let output: Value = replayed.output.expect("the step has an output");
        assert_eq!(output["run_id"], json!(failer.id));
        assert_eq!(output["status"], json!("failed"));
        assert!(output["error"].is_string());
        assert_eq!(seen(&seen_names), vec!["Ada".to_string()]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_without_allow_failure_a_failed_child_still_fails_the_parent() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _seen) = allow_failure_engine(&store);

        let outcome = engine
            .run_handler("strict", TriggerKind::Manual, json!({}))
            .await;
        drop(outcome);

        let parent = run_of(&store, "strict").await;
        assert_eq!(parent.status.state, RunStatus::Failed);
        let step = workflow_step(&store, parent.id, "failer").await;
        assert_eq!(step.status.state, StepStatus::Failed);
        assert_eq!(
            run_of(&store, "failer").await.status.state,
            RunStatus::Failed
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_allow_failure_does_not_tolerate_a_suspension() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, seen_names) = allow_failure_engine(&store);

        let parent = start_chain(&engine, "tolerant-asker").await;
        assert_eq!(parent.status.state, RunStatus::AwaitingApproval);

        let step = workflow_step(&store, parent.id, "asker").await;
        assert_eq!(step.status.state, StepStatus::Running);

        let child = run_of(&store, "asker").await;
        answer(&store, child.id, "Ada").await;
        let result = engine
            .resume_run(child.id)
            .await
            .expect("the chain resumes");

        assert_eq!(result.run.status.state, RunStatus::Completed);
        assert_eq!(
            workflow_step(&store, parent.id, "asker").await.status.state,
            StepStatus::Completed
        );
        assert_eq!(seen(&seen_names), vec!["Ada".to_string()]);
    })
    .await
    .expect("test timed out");
}

// ---- resume_chain: the worker lease follows the root ----

/// Workflow name of [`LeaseProbe`].
const LEASE_PROBE: &str = "lease-probe";

/// Worker that picks the child in the `resume_chain_*` tests.
const WORKER_ID: &str = "worker-1";

/// Leases seen by [`LeaseProbe`] each time its handler body starts.
#[derive(Debug, Clone)]
struct LeaseObservation {
    root_worker_id: Option<String>,
    root_lease_expires_at: Option<DateTime<Utc>>,
    child_worker_id: Option<String>,
    child_lease_expires_at: Option<DateTime<Utc>>,
}

type Observations = Arc<Mutex<Vec<LeaseObservation>>>;

/// A root that records the leases of itself and of its [`Asker`] child, then
/// runs the child and a step.
///
/// The record happens before the `Workflow` step, so on the resumed
/// execution it sees the root and the child right after `resume_chain`
/// moved the root to `Running`, before the child is re-entered. With `hold`,
/// a resumed execution holding a lease notifies it and never goes further,
/// so the root stays `Running` with its lease.
struct LeaseProbe {
    store: Arc<InMemoryStore>,
    observed: Observations,
    seen: Seen,
    hold: Option<Arc<Notify>>,
}

impl WorkflowHandler for LeaseProbe {
    fn name(&self) -> &str {
        LEASE_PROBE
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let root = load_run(&self.store, ctx.run_id()).await;
            let child = runs_of(&self.store, "asker").await.into_iter().next();
            self.observed
                .lock()
                .expect("observed lock")
                .push(LeaseObservation {
                    root_worker_id: root.worker_id.clone(),
                    root_lease_expires_at: root.lease_expires_at,
                    child_worker_id: child.as_ref().and_then(|c| c.worker_id.clone()),
                    child_lease_expires_at: child.as_ref().and_then(|c| c.lease_expires_at),
                });
            if root.worker_id.is_some()
                && let Some(reached) = &self.hold
            {
                reached.notify_one();
                pending::<()>().await;
            }
            ctx.workflow(
                &Asker {
                    seen: self.seen.clone(),
                },
                NoInput {},
            )
            .await?;
            ctx.shell("after-child", ShellConfig::new("echo done"))
                .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for LeaseProbe {
    type Input = NoInput;
}

/// An engine running [`LeaseProbe`] over `store`.
fn lease_probe_engine(
    store: &Arc<InMemoryStore>,
    hold: Option<Arc<Notify>>,
) -> (Arc<Engine>, Observations) {
    let observed = Observations::default();
    let seen = Seen::default();
    let mut engine = new_engine(store);
    engine
        .register(Asker { seen: seen.clone() })
        .expect("register asker");
    engine
        .register(LeaseProbe {
            store: store.clone(),
            observed: observed.clone(),
            seen,
            hold,
        })
        .expect("register lease probe");
    (Arc::new(engine), observed)
}

/// Answer the human input of the child and requeue it to `Pending`, like the
/// API does in `ExecutionMode::Workers`.
async fn answer_and_requeue(store: &InMemoryStore, child_run_id: Uuid) {
    let step = open_input_step(store, child_run_id).await;
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                output: Some(json!({ "name": "Ada" })),
                completed_at: Some(Utc::now()),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await
        .expect("store the answer");
    store
        .update_run_status(child_run_id, RunStatus::Pending)
        .await
        .expect("requeue the child");
}

/// Pick the requeued child like a worker does, with a lease of `ttl`.
async fn pick_child(store: &InMemoryStore, child_run_id: Uuid, ttl: Duration) -> Run {
    let picked = store
        .pick_next_pending(Some(LeaseRequest {
            worker_id: WORKER_ID.to_string(),
            ttl,
        }))
        .await
        .expect("pick")
        .expect("the requeued child is pending");
    assert_eq!(picked.id, child_run_id, "only the child is pending");
    picked
}

/// The last observation of [`LeaseProbe`]: the resumed execution.
fn last_observation(observed: &Observations) -> LeaseObservation {
    observed
        .lock()
        .expect("observed lock")
        .last()
        .cloned()
        .expect("the probe ran")
}

#[tokio::test]
async fn resume_chain_transfers_the_child_lease_to_the_root() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, observed) = lease_probe_engine(&store, None);

        let root = start_chain(&engine, LEASE_PROBE).await;
        assert_eq!(root.status.state, RunStatus::AwaitingApproval);
        let child = run_of(&store, "asker").await;

        answer_and_requeue(&store, child.id).await;
        let picked = pick_child(&store, child.id, Duration::from_secs(60)).await;
        let child_expiry = picked.lease_expires_at.expect("the child holds a lease");

        let result = engine
            .execute_handler_run(child.id)
            .await
            .expect("the chain resumes");
        assert_eq!(result.run.id, root.id);
        assert_eq!(result.run.status.state, RunStatus::Completed);

        let resumed = last_observation(&observed);
        assert_eq!(
            resumed.root_worker_id.as_deref(),
            Some(WORKER_ID),
            "the root takes the worker of the child"
        );
        assert_eq!(
            resumed.root_lease_expires_at,
            Some(child_expiry),
            "the root takes the expiry of the child"
        );

        // A finished run never keeps a lease.
        let root_after = load_run(&store, root.id).await;
        assert!(root_after.worker_id.is_none());
        assert!(root_after.lease_expires_at.is_none());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_chain_releases_the_child_lease() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, observed) = lease_probe_engine(&store, None);

        start_chain(&engine, LEASE_PROBE).await;
        let child = run_of(&store, "asker").await;

        answer_and_requeue(&store, child.id).await;
        pick_child(&store, child.id, Duration::from_secs(60)).await;

        engine
            .execute_handler_run(child.id)
            .await
            .expect("the chain resumes");

        let resumed = last_observation(&observed);
        assert!(
            resumed.child_worker_id.is_none(),
            "the child gave its lease to the root"
        );
        assert!(resumed.child_lease_expires_at.is_none());
        let child_after = load_run(&store, child.id).await;
        assert_eq!(child_after.status.state, RunStatus::Completed);
        assert!(child_after.worker_id.is_none());
        assert!(child_after.lease_expires_at.is_none());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_chain_without_child_lease_leaves_root_unleased() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, observed) = lease_probe_engine(&store, None);

        let root = start_chain(&engine, LEASE_PROBE).await;
        let child = run_of(&store, "asker").await;

        // Resumed in-process (Local mode, API-side resume): no lease anywhere.
        answer(&store, child.id, "Ada").await;
        let result = engine
            .resume_run(child.id)
            .await
            .expect("the chain resumes");
        assert_eq!(result.run.id, root.id);
        assert_eq!(result.run.status.state, RunStatus::Completed);

        let resumed = last_observation(&observed);
        assert!(resumed.root_worker_id.is_none());
        assert!(resumed.root_lease_expires_at.is_none());
        assert!(resumed.child_worker_id.is_none());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_chain_root_lease_is_reaped_when_expired() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let reached = Arc::new(Notify::new());
        let (engine, _observed) = lease_probe_engine(&store, Some(reached.clone()));

        let root = start_chain(&engine, LEASE_PROBE).await;
        let child = run_of(&store, "asker").await;

        answer_and_requeue(&store, child.id).await;
        pick_child(&store, child.id, Duration::from_nanos(1)).await;
        // `Utc::now()` has microsecond resolution: let the lease expire for real.
        sleep(Duration::from_millis(2)).await;

        // The worker died mid-replay: the root holds the expired lease.
        let execution = {
            let engine = engine.clone();
            let child_run_id = child.id;
            spawn(async move { engine.execute_handler_run(child_run_id).await })
        };
        reached.notified().await;

        let reaped = store.reap_expired_leases(10).await.expect("reap");
        execution.abort();

        assert_eq!(reaped.len(), 1, "only the root holds a lease");
        assert_eq!(reaped[0].run.id, root.id);
        assert_eq!(reaped[0].from, RunStatus::Running);
        let expected = if root.max_retries == 0 {
            RunStatus::Failed
        } else {
            RunStatus::Pending
        };
        assert_eq!(reaped[0].to, expected);
        let root_after = load_run(&store, root.id).await;
        assert_eq!(root_after.status.state, expected);
        assert_eq!(root_after.lease_recoveries, 1);
        assert!(root_after.worker_id.is_none());
        let child_after = load_run(&store, child.id).await;
        assert_eq!(child_after.lease_recoveries, 0, "the child is not reaped");
    })
    .await
    .expect("test timed out");
}
