//! Workflows and helpers shared by the signal send tests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{AuditLogSubscriber, Event};
use ironflow_engine::signal::{SentSignals, Signal, SignalDelivery};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunFilter, RunStatus, SignalFilter, Step, StepKind, TriggerKind};
use ironflow_store::signal_store::SignalStore;
use ironflow_store::store::RunStore;

/// Test timeout for bodies that wait for a resumed run.
pub const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow waiting for the human reply.
pub const WAITER: &str = "wait-reply";

/// Workflow sending the human reply.
pub const ROUTER: &str = "route-reply";

/// The signal the waiter waits for and the router sends.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct HumanReplied {
    pub body: String,
}

impl Signal for HumanReplied {
    const NAME: &'static str = "gitlab.human_replied";
}

/// A signal no router declares.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Undeclared {
    pub body: String,
}

impl Signal for Undeclared {
    const NAME: &'static str = "gitlab.undeclared";
}

/// Payload of both workflows: the discussion the reply belongs to.
#[derive(Deserialize)]
struct Discussion {
    discussion_id: String,
}

/// Bodies the waiter received.
pub type Seen = Arc<Mutex<Vec<String>>>;

/// Waits for [`HumanReplied`] on its discussion and records the body.
struct WaitReply {
    seen: Seen,
}

impl WorkflowHandler for WaitReply {
    fn name(&self) -> &str {
        WAITER
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: Discussion = ctx.input().await?;
            let reply = ctx
                .wait_for_signal::<HumanReplied>(
                    "wait-reply",
                    &input.discussion_id,
                    Duration::from_secs(3600),
                )
                .await?
                .ok_or_else(|| EngineError::StepConfig("no reply".to_string()))?;
            self.seen.lock().expect("seen lock").push(reply.body);
            Ok(())
        })
    }
}

/// Sends [`HumanReplied`] on the discussion of its payload, and declares
/// only that signal.
///
/// `send_undeclared` makes it send [`Undeclared`] instead; `key` overrides
/// the payload's discussion.
pub struct RouteReply {
    pub send_undeclared: bool,
    pub key: Option<String>,
    pub deliveries: Arc<Mutex<Vec<SignalDelivery>>>,
}

impl RouteReply {
    pub fn new() -> Self {
        Self {
            send_undeclared: false,
            key: None,
            deliveries: Arc::default(),
        }
    }
}

impl WorkflowHandler for RouteReply {
    fn name(&self) -> &str {
        ROUTER
    }

    fn sent_signals(&self) -> SentSignals {
        SentSignals::new().allow::<HumanReplied>()
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: Discussion = ctx.input().await?;
            let key = self.key.clone().unwrap_or(input.discussion_id);
            let body = "ship it".to_string();
            let delivery = if self.send_undeclared {
                ctx.send_signal("notify", &Undeclared { body }, &key)
                    .await?
            } else {
                ctx.send_signal("notify", &HumanReplied { body }, &key)
                    .await?
            };
            self.deliveries
                .lock()
                .expect("deliveries lock")
                .push(delivery);
            Ok(())
        })
    }
}

pub fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        "/tmp/ironflow-fixtures",
    ))
}

/// An engine running the waiter and `router`, persisting its audit log.
pub fn engine_with(store: &Arc<InMemoryStore>, router: RouteReply) -> (Engine, Seen) {
    let seen = Seen::default();
    let mut engine = Engine::new(store.clone(), provider());
    engine
        .register(WaitReply { seen: seen.clone() })
        .expect("register waiter");
    engine.register(router).expect("register router");
    engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
    (engine, seen)
}

/// Run the waiter until it suspends on `discussion_id`.
pub async fn start_waiting(engine: &Engine, discussion_id: &str) -> Uuid {
    let result = engine
        .run_handler(
            WAITER,
            TriggerKind::Manual,
            json!({ "discussion_id": discussion_id }),
        )
        .await
        .expect("waiter suspends on the signal");
    assert_eq!(result.run.status.state, RunStatus::Sleeping);
    result.run.id
}

/// Run the router on `discussion_id`.
pub async fn route(engine: &Engine, discussion_id: &str) -> Result<Uuid, EngineError> {
    engine
        .run_handler(
            ROUTER,
            TriggerKind::Manual,
            json!({ "discussion_id": discussion_id }),
        )
        .await
        .map(|result| result.run.id)
}

/// Poll the store until the run reaches `status`.
pub async fn wait_for_status(store: &InMemoryStore, run_id: Uuid, status: RunStatus) {
    timeout(TEST_TIMEOUT, async {
        loop {
            let run = store
                .get_run(run_id)
                .await
                .expect("get run")
                .expect("run exists");
            if run.status.state == status {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run never reached the expected status");
}

/// The send steps of a run.
pub async fn send_steps(store: &InMemoryStore, run_id: Uuid) -> Vec<Step> {
    store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .filter(|s| s.kind == StepKind::SignalSend)
        .collect()
}

/// Number of signals stored so far.
pub async fn stored_signals(store: &InMemoryStore) -> u64 {
    store
        .list_signals(SignalFilter::default(), 1, 20)
        .await
        .expect("list signals")
        .total
}

/// The id of the only run of `workflow`.
pub async fn only_run_of(store: &InMemoryStore, workflow: &str) -> Uuid {
    let filter = RunFilter {
        workflow_name: Some(workflow.to_string()),
        ..RunFilter::default()
    };
    let runs = store
        .list_runs(filter, 1, 50)
        .await
        .expect("list runs")
        .items;
    assert_eq!(runs.len(), 1, "exactly one {workflow} run");
    runs[0].id
}
