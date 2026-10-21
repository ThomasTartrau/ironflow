//! Handlers, providers and store helpers shared by the wave tests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Barrier, Notify};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::{
    AgentConfig, AgentOutput, AgentProvider, InvokeFuture, LABEL_ROOT_RUN_ID,
};
use ironflow_engine::config::{
    AgentStepConfig, ApprovalConfig, HumanInputConfig, ShellConfig, WorkflowOptions,
};
use ironflow_engine::context::{PARENT_RUN_ID_LABEL, WorkflowContext};
use ironflow_engine::engine::{Engine, EnqueueOptions};
use ironflow_engine::error::EngineError;
use ironflow_engine::executor::SubWorkflowOutcome;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::notify::{Event, EventSubscriber, SubscriberFuture};
use ironflow_engine::testing::MockAgentProvider;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    LeaseRequest, Run, RunFilter, RunStatus, Step, StepKind, StepStatus, StepUpdate, TriggerKind,
};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for every test body.
pub const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Name of the [`Worker`] handler.
pub const WORKER: &str = "worker";

/// Name of the [`Wave`] handler.
pub const WAVE: &str = "wave";

/// Name of the [`Leaf`] handler.
pub const LEAF: &str = "leaf";

/// Declared budget of a [`Action::Spend`] agent step, and what it costs.
pub const SPEND_USD: f64 = 0.10;

/// What a [`Worker`] child does.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    /// Report its agent as its output.
    Done,
    /// Run one shell step, then report its agent.
    Report,
    /// Fail its handler.
    Fail,
    /// Wait until every child of the wave reached [`Kit::meet`].
    Meet,
    /// Ask a human for a name, reported as its output.
    Ask,
    /// Ask a human for a name, then hold while [`Kit::hold`] is set.
    AskThenHold,
    /// Wait for an approval.
    Approve,
    /// Run one agent step of [`SPEND_USD`]; with `after_first`, only once the
    /// [`GatedProvider`] received its first call.
    Spend { after_first: bool },
    /// Hold while [`Kit::hold`] is set.
    Hold,
    /// Run a [`Leaf`] child of its own.
    Nest,
}

/// Input of [`Worker`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub agent: String,
    pub action: Action,
}

/// The answer an [`Action::Ask`] child waits for.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NameAnswer {
    pub name: String,
}

/// What the [`Worker`] children of a test share with it.
pub struct Kit {
    /// Every [`Action::Meet`] child waits here for its siblings.
    pub meet: Barrier,
    /// Notified on the first call to the [`GatedProvider`].
    pub first_call: Arc<Notify>,
    /// While set, holding children wait on [`Kit::release`].
    pub hold: AtomicBool,
    /// Wakes the holding children.
    pub release: Notify,
    /// How many times a child reached its hold point.
    pub holds: AtomicU32,
    /// How many times a child asked a human.
    pub asks: AtomicU32,
}

impl Kit {
    /// A kit whose barrier waits for `meet` children.
    pub fn new(meet: usize) -> Arc<Self> {
        Arc::new(Self {
            meet: Barrier::new(meet),
            first_call: Arc::new(Notify::new()),
            hold: AtomicBool::new(false),
            release: Notify::new(),
            holds: AtomicU32::new(0),
            asks: AtomicU32::new(0),
        })
    }

    async fn hold_point(&self) {
        self.holds.fetch_add(1, Ordering::SeqCst);
        while self.hold.load(Ordering::SeqCst) {
            self.release.notified().await;
        }
    }

    /// Let every holding child go on.
    pub fn release_all(&self) {
        self.hold.store(false, Ordering::SeqCst);
        self.release.notify_waiters();
    }
}

/// The child every wave of these tests runs.
#[derive(Clone)]
pub struct Worker {
    pub kit: Arc<Kit>,
}

impl WorkflowHandler for Worker {
    fn name(&self) -> &str {
        WORKER
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let task: Task = ctx.input().await?;
            match task.action {
                Action::Done => {}
                Action::Report => {
                    ctx.shell("report", ShellConfig::new("echo reported"))
                        .await?;
                }
                Action::Fail => {
                    return Err(EngineError::InvalidWorkflow(format!(
                        "{} refused",
                        task.agent
                    )));
                }
                Action::Meet => {
                    self.kit.meet.wait().await;
                }
                Action::Ask | Action::AskThenHold => {
                    self.kit.asks.fetch_add(1, Ordering::SeqCst);
                    let answer: NameAnswer = ctx
                        .human_input("ask", HumanInputConfig::new("Who?"))
                        .await?;
                    if matches!(task.action, Action::AskThenHold) {
                        self.kit.hold_point().await;
                    }
                    ctx.set_output(&answer.name)?;
                    return Ok(());
                }
                Action::Approve => {
                    ctx.approval("gate", ApprovalConfig::new("ship it?"))
                        .await?;
                }
                Action::Spend { after_first } => {
                    if after_first {
                        self.kit.first_call.notified().await;
                    }
                    ctx.agent(
                        "spend",
                        AgentStepConfig::new("spend").max_budget_usd(SPEND_USD),
                    )
                    .await?;
                }
                Action::Hold => {
                    self.kit.hold_point().await;
                }
                Action::Nest => {
                    let leaf = Task {
                        agent: task.agent.clone(),
                        action: Action::Done,
                    };
                    ctx.workflow(&Leaf, leaf).await?;
                }
            }
            ctx.set_output(&task.agent)?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Worker {
    type Input = Task;
}

/// The child an [`Action::Nest`] worker runs; does nothing.
pub struct Leaf;

impl WorkflowHandler for Leaf {
    fn name(&self) -> &str {
        LEAF
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

impl TypedWorkflow for Leaf {
    type Input = Task;
}

/// One entry of a wave: `agent` doing `action`, in a step named after it.
pub fn task(agent: &str, action: Action) -> (Task, WorkflowOptions) {
    (
        Task {
            agent: agent.to_string(),
            action,
        },
        WorkflowOptions::new().step_name(format!("delegate-{agent}")),
    )
}

/// What [`Wave`] read from `workflows_with`, once per execution that got
/// past the wave.
pub type Outcomes = Arc<Mutex<Vec<String>>>;

/// One outcome, as `<status>:<output>` or `conflict:<holder run id>`.
fn describe(outcome: &SubWorkflowOutcome) -> Result<String, EngineError> {
    Ok(match outcome {
        SubWorkflowOutcome::Completed(child) => format!(
            "{}:{}",
            child.status(),
            child.output::<String>()?.unwrap_or_default()
        ),
        SubWorkflowOutcome::Conflict(conflict) => format!("conflict:{}", conflict.run_id()),
    })
}

/// Runs one wave of [`Worker`] children, then, with `ask_after`, waits for a
/// human.
pub struct Wave {
    pub worker: Worker,
    pub entries: Vec<(Task, WorkflowOptions)>,
    pub outcomes: Outcomes,
    pub ask_after: bool,
}

impl WorkflowHandler for Wave {
    fn name(&self) -> &str {
        WAVE
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let outcomes = ctx
                .workflows_with(&self.worker, self.entries.clone())
                .await?;
            let seen = outcomes
                .iter()
                .map(describe)
                .collect::<Result<Vec<_>, _>>()?;
            self.outcomes
                .lock()
                .expect("outcomes lock")
                .push(seen.join(","));
            if self.ask_after {
                let _: NameAnswer = ctx
                    .human_input("after-wave", HumanInputConfig::new("Go on?"))
                    .await?;
            }
            Ok(())
        })
    }
}

/// Build an engine running a [`Wave`] of `entries` with the [`Worker`] of `kit`.
pub fn wave_engine(
    store: &Arc<InMemoryStore>,
    provider: Arc<dyn AgentProvider>,
    kit: &Arc<Kit>,
    entries: Vec<(Task, WorkflowOptions)>,
    ask_after: bool,
) -> (Engine, Outcomes) {
    let store_dyn: Arc<dyn Store> = store.clone();
    let mut engine = Engine::new(store_dyn, provider);
    let worker = Worker { kit: kit.clone() };
    let outcomes = Outcomes::default();
    engine.register(worker.clone()).expect("register worker");
    engine.register(Leaf).expect("register leaf");
    engine
        .register(Wave {
            worker,
            entries,
            outcomes: outcomes.clone(),
            ask_after,
        })
        .expect("register wave");
    (engine, outcomes)
}

pub fn mock_provider() -> Arc<dyn AgentProvider> {
    Arc::new(MockAgentProvider::new(|_cfg| {
        Ok(AgentOutput::new(json!("done")))
    }))
}

/// An agent provider whose calls wait until [`GatedProvider::release`], each
/// reporting a cost of [`SPEND_USD`].
pub struct GatedProvider {
    pub calls: AtomicU32,
    first_call: Arc<Notify>,
    release: Notify,
    open: AtomicBool,
}

impl GatedProvider {
    pub fn new(kit: &Kit) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU32::new(0),
            first_call: kit.first_call.clone(),
            release: Notify::new(),
            open: AtomicBool::new(false),
        })
    }

    /// Let every call return.
    pub fn release(&self) {
        self.open.store(true, Ordering::SeqCst);
        self.release.notify_waiters();
    }
}

impl AgentProvider for GatedProvider {
    fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.first_call.notify_one();
            }
            while !self.open.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            let mut output = AgentOutput::new(json!("spent"));
            output.cost_usd = Some(SPEND_USD);
            Ok(output)
        })
    }
}

/// Records every event the engine publishes.
#[derive(Clone, Default)]
pub struct EventCollector {
    pub events: Arc<Mutex<Vec<Event>>>,
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

/// Create a run of `workflow` with `options` and execute it inline.
pub async fn start(
    engine: &Engine,
    store: &InMemoryStore,
    workflow: &str,
    options: EnqueueOptions,
) -> (Uuid, Result<(), EngineError>) {
    let run = engine
        .enqueue_handler_with_options(workflow, TriggerKind::Manual, json!({}), options)
        .await
        .expect("enqueue")
        .into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("mark running");
    let result = engine.execute_handler_run(run.id).await.map(|_| ());
    (run.id, result)
}

pub fn labelled(labels: &[(&str, &str)]) -> EnqueueOptions {
    EnqueueOptions {
        labels: labels
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ..EnqueueOptions::default()
    }
}

pub async fn load_run(store: &InMemoryStore, run_id: Uuid) -> Run {
    store
        .get_run(run_id)
        .await
        .expect("get run")
        .expect("run exists")
}

/// Every run of `workflow`, oldest first.
pub async fn runs_of(store: &InMemoryStore, workflow: &str) -> Vec<Run> {
    let filter = RunFilter {
        workflow_name: Some(workflow.to_string()),
        ..RunFilter::default()
    };
    let mut runs = store
        .list_runs(filter, 1, 100)
        .await
        .expect("list runs")
        .items;
    runs.retain(|r| r.workflow_name == workflow);
    runs.sort_by_key(|r| r.created_at);
    runs
}

/// The run of [`Worker`] started for `agent`.
pub async fn worker_run(store: &InMemoryStore, agent: &str) -> Run {
    let mut runs: Vec<Run> = runs_of(store, WORKER)
        .await
        .into_iter()
        .filter(|r| r.payload["agent"] == agent)
        .collect();
    assert_eq!(runs.len(), 1, "expected one run for {agent}");
    runs.remove(0)
}

/// The `Workflow` steps of `run_id`, in creation order.
pub async fn workflow_steps(store: &InMemoryStore, run_id: Uuid) -> Vec<Step> {
    let mut steps: Vec<Step> = store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .filter(|s| s.kind == StepKind::Workflow)
        .collect();
    steps.sort_by_key(|s| s.created_at);
    steps
}

/// The labels a child run must carry: `expected` plus the two chain labels.
pub fn chain_labels(expected: &[(&str, &str)], parent: Uuid) -> HashMap<String, String> {
    let mut labels: HashMap<String, String> = expected
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    labels.insert(PARENT_RUN_ID_LABEL.to_string(), parent.to_string());
    labels.insert(LABEL_ROOT_RUN_ID.to_string(), parent.to_string());
    labels
}

/// Answer the human input of `run_id` and mark the run running, like the API does.
pub async fn answer(store: &InMemoryStore, run_id: Uuid, name: &str) {
    let step = store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .find(|s| s.kind == StepKind::HumanInput && s.status.state == StepStatus::AwaitingApproval)
        .expect("an open human input");
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

/// Poll until `done` holds.
pub async fn wait_until<F, Fut>(mut done: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    timeout(TEST_TIMEOUT, async {
        while !done().await {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the condition never held");
}

/// Pick the run the way a worker does, with a lease that expires at once.
pub async fn pick_with_expiring_lease(store: &InMemoryStore, run_id: Uuid) {
    let picked = store
        .pick_next_pending(Some(LeaseRequest {
            worker_id: "wave-worker".to_string(),
            ttl: Duration::from_millis(1),
        }))
        .await
        .expect("pick")
        .expect("a pending run");
    assert_eq!(picked.id, run_id);
}
