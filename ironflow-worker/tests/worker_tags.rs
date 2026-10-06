//! Split API + worker deployment: runs are routed to the workers able to take
//! them. A worker only picks runs of the workflows it registered, and only
//! when it carries every tag the run requires; a worker that sends no
//! capabilities (an older worker) still takes everything.
//!
//! Everything is real: the router built by `create_router` served over TCP, an
//! `InMemoryStore` and `Worker`s polling that API.
//!
//! Test names contain `tags` so `cargo test -p ironflow-worker tags` selects
//! the whole suite.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::serve;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use ironflow_worker::{WorkerBuilder, WorkerError};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Workflow registered by the workers of this suite.
const RENDER: &str = "render";

/// Workflow no worker of this suite registers.
const UNKNOWN: &str = "not-registered";

/// Token the workers present to the internal routes.
const WORKER_TOKEN: &str = "test-worker-token";

/// Wall-clock budget for a whole test.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline of every polling loop, generous for a slow CI executor.
const POLL_DEADLINE: Duration = Duration::from_secs(20);

/// Time given to a worker to pick a run it must leave alone: dozens of
/// polls at the 20 ms interval of this suite.
const SETTLE: Duration = Duration::from_millis(500);

/// A workflow that does nothing.
struct Noop(&'static str);

impl WorkflowHandler for Noop {
    fn name(&self) -> &str {
        self.0
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

fn new_run(workflow: &str, worker_tags: &[&str]) -> NewRun {
    NewRun {
        created_by: None,
        workflow_name: workflow.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        idempotency_key: None,
        concurrency_key: None,
        priority: 0,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
        worker_tags: worker_tags.iter().map(|tag| (*tag).to_string()).collect(),
    }
}

/// Create a pending run of `workflow` requiring `worker_tags`.
async fn create_run(store: &InMemoryStore, workflow: &str, worker_tags: &[&str]) -> Uuid {
    store
        .create_run(new_run(workflow, worker_tags))
        .await
        .expect("create run")
        .into_run()
        .id
}

/// Serve the real API router over TCP on top of `store`, in `Workers` mode.
async fn serve_api(store: Arc<InMemoryStore>) -> String {
    serve_state(app_state(store)).await
}

/// The API state over `store`, with an engine in `Workers` mode registering
/// [`RENDER`].
fn app_state(store: Arc<InMemoryStore>) -> AppState {
    let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()))
        .with_execution_mode(ExecutionMode::Workers);
    engine.register(Noop(RENDER)).expect("register on the API");
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-worker-tags".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(16);
    AppState::new(
        store,
        Arc::new(engine),
        jwt_config,
        WORKER_TOKEN.to_string(),
        event_sender,
    )
}

/// Serve the real API router over TCP on top of `state`, and return its
/// base URL.
async fn serve_state(state: AppState) -> String {
    // The limiter keys on the peer address, which `axum::serve` without
    // connect info does not provide.
    let config = RouterConfig {
        rate_limit_auth: None,
        rate_limit_general: None,
        ..RouterConfig::default()
    };
    let router = create_router(state, config);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    spawn(async move {
        serve(listener, router).await.expect("serve");
    });
    format!("http://{addr}")
}

/// Start a worker registering [`RENDER`] and carrying `tags`.
fn spawn_worker(api_url: &str, tags: &[&str]) -> JoinHandle<()> {
    let worker = WorkerBuilder::new(api_url, WORKER_TOKEN)
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(Noop(RENDER))
        .tags(tags.iter().copied())
        .concurrency(2)
        .poll_interval(Duration::from_millis(20))
        .build()
        .expect("build worker");
    spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("worker exited with error: {e:?}");
        }
    })
}

/// Current state of run `id`.
async fn state_of(store: &InMemoryStore, id: Uuid) -> RunStatus {
    store
        .get_run(id)
        .await
        .expect("get run")
        .expect("run exists")
        .status
        .state
}

/// Poll run `id` until it reaches `expected` or [`POLL_DEADLINE`] elapses,
/// and return the last state read.
async fn wait_for_state(store: &InMemoryStore, id: Uuid, expected: RunStatus) -> RunStatus {
    let deadline = Instant::now() + POLL_DEADLINE;
    loop {
        let state = state_of(store, id).await;
        if state == expected || Instant::now() >= deadline {
            return state;
        }
        sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn worker_without_required_tags_leaves_run_pending() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        // Created first: the oldest run must not block the younger one.
        let gpu_run = create_run(&store, RENDER, &["gpu"]).await;
        let plain_run = create_run(&store, RENDER, &[]).await;
        let api_url = serve_api(store.clone()).await;
        let worker = spawn_worker(&api_url, &["arm"]);

        let plain = wait_for_state(&store, plain_run, RunStatus::Completed).await;
        sleep(SETTLE).await;
        let gpu = state_of(&store, gpu_run).await;
        worker.abort();

        assert_eq!(plain, RunStatus::Completed);
        assert_eq!(gpu, RunStatus::Pending, "no worker carries `gpu`");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn worker_with_tags_executes_tagged_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store, RENDER, &["gpu", "region:eu"]).await;
        let api_url = serve_api(store.clone()).await;
        let worker = spawn_worker(&api_url, &["region:eu", "gpu", "arm"]);

        let state = wait_for_state(&store, run, RunStatus::Completed).await;
        worker.abort();

        assert_eq!(state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn worker_skips_workflow_it_does_not_register_tags() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let unknown_run = create_run(&store, UNKNOWN, &[]).await;
        let known_run = create_run(&store, RENDER, &[]).await;
        let api_url = serve_api(store.clone()).await;
        let worker = spawn_worker(&api_url, &[]);

        let known = wait_for_state(&store, known_run, RunStatus::Completed).await;
        sleep(SETTLE).await;
        let unknown = state_of(&store, unknown_run).await;
        worker.abort();

        assert_eq!(known, RunStatus::Completed);
        assert_eq!(
            unknown,
            RunStatus::Pending,
            "the worker did not register `{UNKNOWN}`"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn legacy_pick_without_tags_takes_tagged_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store, UNKNOWN, &["gpu"]).await;
        let api_url = serve_api(store.clone()).await;

        // An older worker sends neither `workflows` nor `tags`.
        let resp = Client::new()
            .get(format!("{api_url}/api/v1/internal/runs/next"))
            .bearer_auth(WORKER_TOKEN)
            .query(&[("worker_id", "legacy-worker"), ("lease_ttl_secs", "60")])
            .send()
            .await
            .expect("pick request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.expect("json body");

        assert_eq!(body["data"]["id"], json!(run.to_string()), "{body}");
        assert_eq!(state_of(&store, run).await, RunStatus::Running);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn pick_with_empty_tags_skips_tagged_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store, RENDER, &["gpu"]).await;
        let api_url = serve_api(store.clone()).await;

        // A worker carrying no tag sends an empty `tags`.
        let resp = Client::new()
            .get(format!("{api_url}/api/v1/internal/runs/next"))
            .bearer_auth(WORKER_TOKEN)
            .query(&[("worker_id", "untagged-worker"), ("tags", "")])
            .send()
            .await
            .expect("pick request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.expect("json body");

        assert_eq!(body["data"], Value::Null, "{body}");
        assert_eq!(state_of(&store, run).await, RunStatus::Pending);
    })
    .await
    .expect("test timed out");
}

/// The pending count a worker built with `tags` reads from the API: the
/// query carries its capabilities exactly as the queue depth gauge sends
/// them.
async fn queue_depth_for(api_url: &str, tags: &[&str]) -> u64 {
    let worker = WorkerBuilder::new(api_url, WORKER_TOKEN)
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(Noop(RENDER))
        .tags(tags.iter().copied())
        .build()
        .expect("build worker");
    let capabilities = worker.capabilities();
    let mut query = vec![("tags", capabilities.tags.join(","))];
    if let Some(workflows) = &capabilities.workflows {
        query.push(("workflows", workflows.join(",")));
    }

    let resp = Client::new()
        .get(format!("{api_url}/api/v1/internal/runs/pending-count"))
        .bearer_auth(WORKER_TOKEN)
        .query(&query)
        .send()
        .await
        .expect("pending count request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json body");
    body["data"]["pending_runs"]
        .as_u64()
        .unwrap_or_else(|| panic!("pending_runs missing: {body}"))
}

#[tokio::test]
async fn queue_depth_route_counts_only_runs_matching_worker_tags() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        for _ in 0..3 {
            create_run(&store, RENDER, &[]).await;
        }
        for _ in 0..2 {
            create_run(&store, RENDER, &["gpu"]).await;
        }
        create_run(&store, RENDER, &["arm"]).await;
        // A workflow the worker does not register never counts.
        create_run(&store, UNKNOWN, &[]).await;
        let api_url = serve_api(store).await;

        assert_eq!(queue_depth_for(&api_url, &[]).await, 3);
        assert_eq!(queue_depth_for(&api_url, &["gpu"]).await, 5);
        assert_eq!(queue_depth_for(&api_url, &["gpu", "arm"]).await, 6);
    })
    .await
    .expect("test timed out");
}

#[test]
fn builder_rejects_invalid_tags() {
    let result = WorkerBuilder::new("http://localhost:3000", WORKER_TOKEN)
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .tags(["gpu", "not a tag"])
        .build();

    let Err(err) = result else {
        panic!("a tag with spaces must be rejected");
    };
    assert!(matches!(err, WorkerError::Engine(_)), "{err:?}");
    assert!(err.to_string().contains("not a tag"), "{err}");
}

/// `ironflow_worker_queue_depth` only counts the runs the worker can take:
/// the worker sends its workflows and tags to `GET /runs/pending-count`.
///
/// The Prometheus recorder is global, and the workers of the other tests of
/// this binary publish the same gauge: the counts expected here (3 and 5)
/// are ones those workers, which see at most two pending runs, never report.
#[cfg(feature = "prometheus")]
mod queue_depth {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use chrono::{TimeDelta, Utc};
    use ironflow_core::metric_names::WORKER_QUEUE_DEPTH;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::RunStore;
    use tokio::time::{sleep, timeout};

    use super::{RENDER, TEST_TIMEOUT, UNKNOWN, app_state, new_run, serve_state, spawn_worker};

    /// Past the 5 s refresh period of the gauge, so a missing value is not a
    /// timing artifact.
    const GAUGE_DEADLINE: Duration = Duration::from_secs(8);

    /// Create a run of `workflow` requiring `worker_tags`, scheduled an hour
    /// ahead so no worker executes it and it stays pending.
    async fn create_future_run(store: &InMemoryStore, workflow: &str, worker_tags: &[&str]) {
        let mut run = new_run(workflow, worker_tags);
        run.scheduled_at = Some(Utc::now() + TimeDelta::hours(1));
        store.create_run(run).await.expect("create pending run");
    }

    /// The value of the bare metric `name` in a Prometheus text exposition.
    fn gauge_value(exposition: &str, name: &str) -> Option<f64> {
        exposition
            .lines()
            .filter(|line| !line.starts_with('#'))
            .find_map(|line| {
                let (series, value) = line.rsplit_once(' ')?;
                (series == name)
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
    }

    /// Poll the queue depth gauge until it reads `expected` or
    /// [`GAUGE_DEADLINE`] elapses, and return the last value read.
    async fn wait_for_depth(render: &impl Fn() -> String, expected: f64) -> Option<f64> {
        let deadline = Instant::now() + GAUGE_DEADLINE;
        let mut value = None;
        while Instant::now() < deadline {
            value = gauge_value(&render(), WORKER_QUEUE_DEPTH);
            if value == Some(expected) {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        value
    }

    #[tokio::test]
    async fn queue_depth_counts_only_runs_matching_worker_tags() {
        timeout(TEST_TIMEOUT, async {
            let store = Arc::new(InMemoryStore::new());
            for _ in 0..3 {
                create_future_run(&store, RENDER, &[]).await;
            }
            for _ in 0..2 {
                create_future_run(&store, RENDER, &["gpu"]).await;
            }
            // A workflow the workers do not register never counts.
            create_future_run(&store, UNKNOWN, &[]).await;

            let state = app_state(store);
            let prometheus = state.prometheus_handle.clone();
            let render = move || prometheus.render();
            let api_url = serve_state(state).await;

            // Six pending runs: an untagged worker can take the three that
            // require no tag.
            let untagged = spawn_worker(&api_url, &[]);
            let depth = wait_for_depth(&render, 3.0).await;
            untagged.abort();
            assert_eq!(
                depth,
                Some(3.0),
                "expected {WORKER_QUEUE_DEPTH} 3, exposition was:\n{}",
                render()
            );

            // A `gpu` worker also counts the two `gpu` runs.
            let gpu = spawn_worker(&api_url, &["gpu"]);
            let depth = wait_for_depth(&render, 5.0).await;
            gpu.abort();
            assert_eq!(
                depth,
                Some(5.0),
                "expected {WORKER_QUEUE_DEPTH} 5, exposition was:\n{}",
                render()
            );
        })
        .await
        .expect("test timed out");
    }
}
