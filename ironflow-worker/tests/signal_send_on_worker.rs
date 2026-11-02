//! Split API + worker deployment: a run executed by a worker sends a signal
//! with `ctx.send_signal`, and wakes another run waiting for it, without any
//! API key.
//!
//! Everything is real: the router built by `create_router` served over TCP, an
//! `InMemoryStore`, an `Engine` in `ExecutionMode::Workers` on the API side
//! with the audit log subscriber, and a `Worker` polling that API. Both runs
//! are executed by the worker.
//!
//! Test names start with `signal_send` so `cargo test -p ironflow-worker
//! signal_send` selects the whole suite.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::serve;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{AuditLogSubscriber, Event};
use ironflow_engine::signal::{SentSignals, Signal};
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    AuditLogEntry, AuditLogFilter, EventKind, RunStatus, SignalFilter, Step, StepKind, StepStatus,
    TriggerKind,
};
use ironflow_store::signal_store::SignalStore;
use ironflow_store::store::RunStore;
use ironflow_worker::WorkerBuilder;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Workflow waiting for the human reply.
const WAITER: &str = "wait-reply-on-worker";

/// Workflow sending the human reply.
const ROUTER: &str = "route-reply-on-worker";

/// Wall-clock budget for a whole test.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline of every polling loop, generous for a slow CI executor.
const POLL_DEADLINE: Duration = Duration::from_secs(20);

/// The signal the waiter waits for and the router sends.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct HumanReplied {
    body: String,
}

impl Signal for HumanReplied {
    const NAME: &'static str = "gitlab.human_replied";
}

/// A signal the router does not declare.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Undeclared {
    body: String,
}

impl Signal for Undeclared {
    const NAME: &'static str = "gitlab.undeclared";
}

/// Payload of both workflows.
#[derive(Deserialize)]
struct Discussion {
    discussion_id: String,
    #[serde(default)]
    undeclared: bool,
}

/// Waits for [`HumanReplied`] on its discussion.
struct WaitReply;

impl WorkflowHandler for WaitReply {
    fn name(&self) -> &str {
        WAITER
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: Discussion = ctx.input().await?;
            ctx.wait_for_signal::<HumanReplied>(
                "wait-reply",
                &input.discussion_id,
                Duration::from_secs(3600),
            )
            .await?
            .ok_or_else(|| EngineError::StepConfig("no reply".to_string()))?;
            Ok(())
        })
    }
}

/// Sends [`HumanReplied`], or [`Undeclared`] when the payload asks for it.
struct RouteReply;

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
            let body = "ship it".to_string();
            let key = &input.discussion_id;
            if input.undeclared {
                ctx.send_signal("notify", &Undeclared { body }, key).await?;
            } else {
                ctx.send_signal("notify", &HumanReplied { body }, key)
                    .await?;
            }
            Ok(())
        })
    }
}

/// A running API server and worker; the worker stops when this is dropped.
struct Harness {
    state: AppState,
    store: Arc<InMemoryStore>,
    worker: JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

impl Harness {
    async fn start() -> Self {
        let store = Arc::new(InMemoryStore::new());

        let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()))
            .with_execution_mode(ExecutionMode::Workers);
        engine.register(WaitReply).expect("register waiter");
        engine.register(RouteReply).expect("register router");
        engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-signal-send-on-worker".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(16);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        );

        // The limiter keys on the peer address, which `axum::serve` without
        // connect info does not provide.
        let config = RouterConfig {
            rate_limit_auth: None,
            rate_limit_general: None,
            ..RouterConfig::default()
        };
        let router = create_router(state.clone(), config);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        spawn(async move {
            serve(listener, router).await.expect("serve");
        });

        let worker = WorkerBuilder::new(&format!("http://{addr}"), "test-worker-token")
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .register(WaitReply)
            .register(RouteReply)
            .worker_id("worker-signal-send")
            .concurrency(1)
            .poll_interval(Duration::from_millis(20))
            .lease_ttl(Duration::from_secs(5))
            .lease_refresh_interval(Duration::from_millis(500))
            .run_timeout(Duration::from_secs(10))
            .build()
            .expect("build worker");
        let worker = spawn(async move {
            if let Err(e) = worker.run().await {
                eprintln!("worker exited with error: {e:?}");
            }
        });

        Self {
            state,
            store,
            worker,
        }
    }

    /// Enqueues a run of `workflow`; the worker claims it.
    async fn enqueue(&self, workflow: &str, discussion_id: &str, undeclared: bool) -> Uuid {
        self.state
            .engine
            .enqueue_handler(
                workflow,
                TriggerKind::Manual,
                json!({"discussion_id": discussion_id, "undeclared": undeclared}),
                0,
            )
            .await
            .expect("enqueue run")
            .id
    }

    /// Polls until the run reaches `expected`, panicking at the deadline.
    async fn wait_for_status(&self, run_id: Uuid, expected: RunStatus) {
        let deadline = Instant::now() + POLL_DEADLINE;
        let mut status = RunStatus::Pending;
        while Instant::now() < deadline {
            status = self
                .store
                .get_run(run_id)
                .await
                .expect("get run")
                .expect("run exists")
                .status
                .state;
            if status == expected {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("run {run_id} did not reach {expected:?} in time, last status {status:?}");
    }

    /// The single send step of `run_id`.
    async fn send_step(&self, run_id: Uuid) -> Step {
        let mut steps: Vec<_> = self
            .store
            .list_steps(run_id)
            .await
            .expect("list steps")
            .into_iter()
            .filter(|s| s.kind == StepKind::SignalSend)
            .collect();
        assert_eq!(steps.len(), 1, "steps: {steps:?}");
        steps.remove(0)
    }

    /// Polls the audit log for the `signal_received` entry of `run_id`.
    async fn signal_audit_entry(&self, run_id: Uuid) -> AuditLogEntry {
        let filter = AuditLogFilter {
            event_type: Some(EventKind::SignalReceived),
            run_id: Some(run_id),
            ..AuditLogFilter::default()
        };
        let deadline = Instant::now() + POLL_DEADLINE;
        while Instant::now() < deadline {
            let page = self
                .store
                .list_audit_logs(filter.clone(), 1, 20)
                .await
                .expect("list audit logs");
            if let Some(entry) = page.items.into_iter().next() {
                return entry;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("no signal_received audit entry for run {run_id}");
    }

    async fn stored_signals(&self) -> u64 {
        self.store
            .list_signals(SignalFilter::default(), 1, 20)
            .await
            .expect("list signals")
            .total
    }
}

#[tokio::test]
async fn signal_send_on_a_worker_wakes_the_waiting_run() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::start().await;
        let waiting = harness.enqueue(WAITER, "d-1", false).await;
        harness.wait_for_status(waiting, RunStatus::Sleeping).await;

        let router = harness.enqueue(ROUTER, "d-1", false).await;
        harness.wait_for_status(router, RunStatus::Completed).await;
        harness.wait_for_status(waiting, RunStatus::Completed).await;

        let step = harness.send_step(router).await;
        assert_eq!(step.status.state, StepStatus::Completed);
        let output = step.output.as_ref().expect("send step has an output");
        assert_eq!(output["resumed"][0]["run_id"], json!(waiting));
        assert_eq!(harness.stored_signals().await, 1);

        let entry = harness.signal_audit_entry(router).await;
        assert_eq!(entry.run_id, Some(router));
        assert_eq!(entry.step_id, Some(step.id));
        assert_eq!(entry.payload["name"], json!(HumanReplied::NAME));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn signal_send_on_a_worker_refuses_an_undeclared_signal() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::start().await;

        let router = harness.enqueue(ROUTER, "d-2", true).await;
        harness.wait_for_status(router, RunStatus::Failed).await;

        let step = harness.send_step(router).await;
        assert_eq!(step.status.state, StepStatus::Failed);
        let error = step.error.as_deref().expect("failed step has an error");
        assert!(error.contains(Undeclared::NAME), "{error}");
        assert_eq!(harness.stored_signals().await, 0);
    })
    .await
    .expect("test timed out");
}
