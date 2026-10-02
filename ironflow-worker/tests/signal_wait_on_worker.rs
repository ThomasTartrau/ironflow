//! Split API + worker deployment: a run executed by a worker suspends on
//! `ctx.wait_for_signal`, and is woken by a signal sent through the API or by
//! its deadline.
//!
//! Everything is real: the router built by `create_router` served over TCP, an
//! `InMemoryStore`, an `Engine` in `ExecutionMode::Workers` on the API side and
//! a `Worker` polling that API. The first execution is done by the worker, so
//! the signal step is created and suspended over HTTP.
//!
//! Test names contain `signal` so `cargo test -p ironflow-worker signal`
//! selects the whole suite.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::serve;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_engine::signal::Signal;
use ironflow_engine::wake::RunWaker;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepKind, StepStatus, TriggerKind};
use ironflow_store::store::RunStore;
use ironflow_worker::WorkerBuilder;
use reqwest::{Client, Response, StatusCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Workflow name registered on both the API and the worker.
const WORKFLOW: &str = "wait-ci-on-worker";

/// Wall-clock budget for a whole test.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline of every polling loop, generous for a slow CI executor.
const POLL_DEADLINE: Duration = Duration::from_secs(20);

/// Pipeline status sent by the CI.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PipelineFinished {
    status: String,
}

impl Signal for PipelineFinished {
    const NAME: &'static str = "ci.pipeline_finished";
}

/// The run payload: the commit to wait on, and for how long.
#[derive(Deserialize)]
struct Input {
    sha: String,
    timeout_ms: u64,
}

/// What the handler received: the pipeline status, or `None` on timeout.
type Seen = Arc<Mutex<Vec<Option<String>>>>;

/// Waits for [`PipelineFinished`] on the payload's commit and records it.
struct WaitCi {
    seen: Seen,
}

impl WorkflowHandler for WaitCi {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: Input = ctx.input().await?;
            let finished = ctx
                .wait_for_signal::<PipelineFinished>(
                    "wait-ci",
                    &input.sha,
                    Duration::from_millis(input.timeout_ms),
                )
                .await?;
            self.seen
                .lock()
                .expect("seen lock")
                .push(finished.map(|f| f.status));
            Ok(())
        })
    }
}

/// A running API server and worker; the worker stops when this is dropped.
struct Harness {
    base_url: String,
    state: AppState,
    store: Arc<InMemoryStore>,
    token: AccessToken,
    seen: Seen,
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
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));

        let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()))
            .with_execution_mode(ExecutionMode::Workers);
        engine
            .register(WaitCi { seen: seen.clone() })
            .expect("register handler");
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-signal-wait-on-worker".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(16);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config.clone(),
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
        let base_url = format!("http://{addr}");

        let worker = WorkerBuilder::new(&base_url, "test-worker-token")
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .register(WaitCi { seen: seen.clone() })
            .worker_id("worker-signal-wait")
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

        let token =
            AccessToken::for_user(Uuid::now_v7(), "admin", true, &jwt_config).expect("token");

        Self {
            base_url,
            state,
            store,
            token,
            seen,
            worker,
        }
    }

    /// Enqueues a run waiting on `sha` for `timeout_ms`; the worker claims it.
    async fn enqueue(&self, sha: &str, timeout_ms: u64) -> Uuid {
        self.state
            .engine
            .enqueue_handler(
                WORKFLOW,
                TriggerKind::Manual,
                json!({"sha": sha, "timeout_ms": timeout_ms}),
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

    /// Sends a signal through the API and returns the response.
    async fn send_signal(&self, key: &str, payload: Value) -> Response {
        Client::new()
            .post(format!("{}/api/v1/signals", self.base_url))
            .bearer_auth(&self.token.0)
            .json(&json!({"name": PipelineFinished::NAME, "key": key, "payload": payload}))
            .send()
            .await
            .expect("post signal")
    }

    fn seen(&self) -> Vec<Option<String>> {
        self.seen.lock().expect("seen lock").clone()
    }
}

#[tokio::test]
async fn signal_resumes_a_run_executed_by_a_worker() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::start().await;
        let run_id = harness.enqueue("abc123", 3_600_000).await;

        harness.wait_for_status(run_id, RunStatus::Sleeping).await;
        assert!(harness.seen().is_empty());
        let run = harness
            .store
            .get_run(run_id)
            .await
            .expect("get run")
            .expect("run exists");
        assert!(run.scheduled_at.is_some(), "sleeping run has a deadline");
        let steps = harness.store.list_steps(run_id).await.expect("list steps");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].kind, StepKind::Signal);
        assert_eq!(steps[0].status.state, StepStatus::Running);
        let step_id = steps[0].id;

        let resp = harness
            .send_signal("abc123", json!({"status": "success"}))
            .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.expect("json body");
        assert_eq!(body["data"]["duplicate"], json!(false));
        let resumed = body["data"]["resumed"].as_array().expect("resumed array");
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0]["run_id"], json!(run_id.to_string()));
        assert_eq!(resumed[0]["step_id"], json!(step_id.to_string()));
        assert!(
            body["data"]["rejected"]
                .as_array()
                .expect("rejected array")
                .is_empty()
        );

        harness.wait_for_status(run_id, RunStatus::Completed).await;
        assert_eq!(harness.seen(), vec![Some("success".to_string())]);

        let steps = harness.store.list_steps(run_id).await.expect("list steps");
        let signal_steps: Vec<_> = steps
            .iter()
            .filter(|s| s.kind == StepKind::Signal)
            .collect();
        assert_eq!(signal_steps.len(), 1, "the signal step was duplicated");
        let step = signal_steps[0];
        assert_eq!(step.id, step_id);
        assert_eq!(step.status.state, StepStatus::Completed);
        let output = step.output.clone().expect("resolved step has an output");
        assert_eq!(output["timed_out"], json!(false));
        assert_eq!(output["payload"], json!({"status": "success"}));
        assert_eq!(output["signal_id"], body["data"]["signal_id"]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn signal_wait_times_out_on_a_worker_and_the_handler_receives_none() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::start().await;
        let run_id = harness.enqueue("slow", 100).await;

        harness.wait_for_status(run_id, RunStatus::Sleeping).await;

        // The waker only claims runs whose deadline has passed: loop until it
        // does. In workers mode it requeues the run instead of resuming it.
        let waker = RunWaker::new(harness.state.engine.clone());
        let deadline = Instant::now() + POLL_DEADLINE;
        let mut woken = Vec::new();
        while woken.is_empty() && Instant::now() < deadline {
            woken = waker.tick().await.expect("tick");
            sleep(Duration::from_millis(20)).await;
        }
        let woken_ids: Vec<Uuid> = woken.iter().map(|r| r.id).collect();
        assert_eq!(woken_ids, vec![run_id]);

        harness.wait_for_status(run_id, RunStatus::Completed).await;
        assert_eq!(harness.seen(), vec![None]);

        let steps = harness.store.list_steps(run_id).await.expect("list steps");
        let signal_steps: Vec<_> = steps
            .iter()
            .filter(|s| s.kind == StepKind::Signal)
            .collect();
        assert_eq!(signal_steps.len(), 1, "the signal step was duplicated");
        assert_eq!(signal_steps[0].status.state, StepStatus::Completed);
        assert_eq!(signal_steps[0].output, Some(json!({"timed_out": true})));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn signal_with_a_mismatched_payload_leaves_the_worker_run_sleeping() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::start().await;
        let run_id = harness.enqueue("bad", 3_600_000).await;

        harness.wait_for_status(run_id, RunStatus::Sleeping).await;

        let resp = harness.send_signal("bad", json!({"status": 42})).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.expect("json body");
        assert!(
            body["data"]["resumed"]
                .as_array()
                .expect("resumed array")
                .is_empty()
        );
        let rejected = body["data"]["rejected"].as_array().expect("rejected array");
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0]["run_id"], json!(run_id.to_string()));

        let run = harness
            .store
            .get_run(run_id)
            .await
            .expect("get run")
            .expect("run exists");
        assert_eq!(run.status.state, RunStatus::Sleeping);
        assert!(harness.seen().is_empty());
    })
    .await
    .expect("test timed out");
}
