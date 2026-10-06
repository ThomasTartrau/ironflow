//! Handlers and helpers shared by the cancellation tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::spawn;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::{HumanInputConfig, ShellConfig, WorkflowOptions};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::executor::SubWorkflowOutcome;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::notify::{Event, EventSubscriber, SubscriberFuture};
use ironflow_store::error::StoreError;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    NewRun, Run, RunFilter, RunStatus, Step, StepKind, StepStatus, TriggerKind,
};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that spawn processes.
pub const TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Concurrency key held by the child of every [`Host`].
pub const KEY: &str = "issue:169";

/// Payload of a workflow that takes nothing.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct NoInput {}

/// A child whose shell step outlives the test unless it is stopped.
struct Stuck;

impl WorkflowHandler for Stuck {
    fn name(&self) -> &str {
        "stuck"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("hang", ShellConfig::new("sleep 60")).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Stuck {
    type Input = NoInput;
}

/// A child that runs one short step, long enough to be cancelled meanwhile.
struct Brief;

impl WorkflowHandler for Brief {
    fn name(&self) -> &str {
        "brief"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("pause", ShellConfig::new("sleep 1")).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Brief {
    type Input = NoInput;
}

/// A child that asks a human, suspending its whole chain.
struct Asker;

impl WorkflowHandler for Asker {
    fn name(&self) -> &str {
        "asker"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let _: Value = ctx
                .human_input("ask", HumanInputConfig::new("Who ships it?"))
                .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Asker {
    type Input = NoInput;
}

/// Which child a [`Host`] starts.
#[derive(Clone, Copy)]
enum ChildKind {
    Stuck,
    Brief,
    Asker,
}

/// Starts one child under [`KEY`], tolerating its failure when `tolerant`,
/// and records the status its step reported.
struct Host {
    name: &'static str,
    child: ChildKind,
    tolerant: bool,
    reported: Arc<Mutex<Option<RunStatus>>>,
}

impl WorkflowHandler for Host {
    fn name(&self) -> &str {
        self.name
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let mut options = WorkflowOptions::new().concurrency_key(KEY);
            if self.tolerant {
                options = options.allow_failure();
            }
            let outcome = match self.child {
                ChildKind::Stuck => ctx.workflow_with(&Stuck, NoInput {}, options).await?,
                ChildKind::Brief => ctx.workflow_with(&Brief, NoInput {}, options).await?,
                ChildKind::Asker => ctx.workflow_with(&Asker, NoInput {}, options).await?,
            };
            if let SubWorkflowOutcome::Completed(child) = outcome {
                *self.reported.lock().expect("reported lock") = Some(child.status());
            }
            Ok(())
        })
    }
}

impl TypedWorkflow for Host {
    type Input = NoInput;
}

/// Runs a strict `asking-host` as a sub-workflow: a three-run chain.
struct Grandparent;

impl WorkflowHandler for Grandparent {
    fn name(&self) -> &str {
        "grandparent"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let host = Host {
                name: "asking-host",
                child: ChildKind::Asker,
                tolerant: false,
                reported: Arc::default(),
            };
            ctx.workflow(&host, NoInput {}).await?;
            Ok(())
        })
    }
}

/// Records every event it receives.
#[derive(Clone, Default)]
pub struct EventCollector {
    events: Arc<Mutex<Vec<Event>>>,
}

impl EventSubscriber for EventCollector {
    fn name(&self) -> &str {
        "event-collector"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        let events = self.events.clone();
        let event = event.clone();
        Box::pin(async move {
            events.lock().expect("collector lock").push(event);
        })
    }
}

impl EventCollector {
    /// Runs that were reported moving to `Cancelled`, in order.
    pub fn cancelled(&self) -> Vec<Uuid> {
        self.events
            .lock()
            .expect("collector lock")
            .iter()
            .filter_map(|e| match e {
                Event::RunStatusChanged(c) if c.to == RunStatus::Cancelled => Some(c.run_id),
                _ => None,
            })
            .collect()
    }

    /// Poll until `run_id` was reported cancelled.
    pub async fn wait_for_cancelled(&self, run_id: Uuid) {
        timeout(TEST_TIMEOUT, async {
            while !self.cancelled().contains(&run_id) {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no cancellation event for run {run_id}"));
    }
}

/// Everything a test needs: the engine, its store, what the hosts saw.
///
/// Registered roots: `stuck-host`, `brief-host`, `tolerant-brief-host`,
/// `asking-host`, `tolerant-asking-host` and `grandparent`.
pub struct Fixture {
    pub store: Arc<InMemoryStore>,
    pub engine: Arc<Engine>,
    pub reported: Arc<Mutex<Option<RunStatus>>>,
    pub events: EventCollector,
}

pub fn fixture(mode: ExecutionMode) -> Fixture {
    let store = Arc::new(InMemoryStore::new());
    let provider: Arc<dyn AgentProvider> = Arc::new(ClaudeCodeProvider::new());
    let dyn_store: Arc<dyn Store> = store.clone();
    let mut engine = Engine::new(dyn_store, provider).with_execution_mode(mode);
    let reported = Arc::new(Mutex::new(None));
    for (name, child, tolerant) in [
        ("stuck-host", ChildKind::Stuck, false),
        ("brief-host", ChildKind::Brief, false),
        ("tolerant-brief-host", ChildKind::Brief, true),
        ("asking-host", ChildKind::Asker, false),
        ("tolerant-asking-host", ChildKind::Asker, true),
    ] {
        engine
            .register(Host {
                name,
                child,
                tolerant,
                reported: reported.clone(),
            })
            .expect("register host");
    }
    engine.register(Stuck).expect("register stuck");
    engine.register(Brief).expect("register brief");
    engine.register(Asker).expect("register asker");
    engine.register(Grandparent).expect("register grandparent");
    let events = EventCollector::default();
    engine.subscribe(events.clone(), &[Event::RUN_STATUS_CHANGED]);
    Fixture {
        store,
        engine: Arc::new(engine),
        reported,
        events,
    }
}

impl Fixture {
    /// Run `workflow` inline until it suspends, returning the root run.
    pub async fn start_suspended(&self, workflow: &str) -> Run {
        let run = self
            .engine
            .run_handler(workflow, TriggerKind::Manual, json!({}))
            .await
            .expect("the chain suspends")
            .run;
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);
        run
    }

    /// Create a run of `workflow` already picked up, as a worker sees it.
    pub async fn picked_run(&self, workflow: &str, max_retries: u32) -> Run {
        let run = self
            .store
            .create_run(new_run(workflow, max_retries, None))
            .await
            .expect("create run")
            .into_run();
        self.store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .expect("pick the run");
        run
    }

    pub async fn status(&self, run_id: Uuid) -> RunStatus {
        self.run(run_id).await.status.state
    }

    pub async fn run(&self, run_id: Uuid) -> Run {
        self.store
            .get_run(run_id)
            .await
            .expect("get run")
            .expect("run exists")
    }

    /// The single run of exactly `workflow`, once it exists.
    pub async fn wait_for_run_of(&self, workflow: &str) -> Run {
        timeout(TEST_TIMEOUT, async {
            loop {
                let filter = RunFilter {
                    workflow_name: Some(workflow.to_string()),
                    ..RunFilter::default()
                };
                let runs = self
                    .store
                    .list_runs(filter, 1, 50)
                    .await
                    .expect("list runs");
                if let Some(run) = runs.items.into_iter().find(|r| r.workflow_name == workflow) {
                    return run;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the run was never created")
    }

    /// Poll until the run reaches `status`.
    pub async fn wait_for_status(&self, run_id: Uuid, status: RunStatus) {
        timeout(TEST_TIMEOUT, async {
            while self.status(run_id).await != status {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("run {run_id} never reached {status}"));
    }

    /// Poll until the run has a step `name` in `status`.
    pub async fn wait_for_step(&self, run_id: Uuid, name: &str, status: StepStatus) {
        timeout(TEST_TIMEOUT, async {
            loop {
                let steps = self.steps(run_id).await;
                if steps
                    .iter()
                    .any(|s| s.name == name && s.status.state == status)
                {
                    return;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("step {name} of run {run_id} never reached {status}"));
    }

    pub async fn steps(&self, run_id: Uuid) -> Vec<Step> {
        self.store.list_steps(run_id).await.expect("list steps")
    }

    /// Steps of `run_id` that are not terminal.
    pub async fn open_steps(&self, run_id: Uuid) -> Vec<Step> {
        let mut steps = self.steps(run_id).await;
        steps.retain(|s| !s.status.state.is_terminal());
        steps
    }

    /// The last `Workflow` step of `run_id` that started `child`.
    pub async fn workflow_step(&self, run_id: Uuid, child: &str) -> Step {
        self.steps(run_id)
            .await
            .into_iter()
            .rev()
            .find(|s| s.kind == StepKind::Workflow && s.name == child)
            .expect("the workflow step exists")
    }

    /// Whether a new run can take [`KEY`].
    pub async fn key_is_free(&self) -> bool {
        match self.store.create_run(new_run("other", 0, Some(KEY))).await {
            Ok(_) => true,
            Err(StoreError::ConcurrencyConflict { .. }) => false,
            Err(other) => panic!("unexpected store error: {other}"),
        }
    }

    /// Start `stuck-host` as a picked run, wait until its child hangs, then
    /// drop the execution the way a worker timeout or panic does.
    pub async fn abandon_with_stuck_child(&self, max_retries: u32) -> (Run, Run) {
        let root = self.picked_run("stuck-host", max_retries).await;
        let engine = self.engine.clone();
        let execution = spawn(async move { engine.execute_handler_run(root.id).await });

        let child = self.wait_for_run_of("stuck").await;
        self.wait_for_step(child.id, "hang", StepStatus::Running)
            .await;
        execution.abort();
        let _ = execution.await;

        assert_eq!(
            self.status(child.id).await,
            RunStatus::Running,
            "an abandoned execution leaves its child running"
        );
        assert!(!self.key_is_free().await);
        (root, child)
    }
}

fn new_run(workflow: &str, max_retries: u32, concurrency_key: Option<&str>) -> NewRun {
    NewRun {
        workflow_name: workflow.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: concurrency_key.map(str::to_string),
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
        worker_tags: Vec::new(),
    }
}
