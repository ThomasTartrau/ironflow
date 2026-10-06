//! A worker polling the API publishes `ironflow_worker_queue_depth`: the number
//! of `Pending` runs, and `ironflow_worker_queue_blocked_runs`: the due runs
//! held back by each saturated concurrency group, both read from the API server
//! that owns the store.
//!
//! Everything is real: the router built by `create_router` served over TCP, an
//! `InMemoryStore` and a `Worker` polling that API. The API and the worker share
//! this process, hence one global Prometheus recorder: the one `AppState`
//! installs, read back through its handle. The tests publish to that same
//! recorder, so they hold [`SERIAL`] to run one at a time.

#![cfg(feature = "prometheus")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::serve;
use chrono::{TimeDelta, Utc};
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::metric_names::{WORKER_QUEUE_BLOCKED_RUNS, WORKER_QUEUE_DEPTH};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::notify::Event;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{ConcurrencyLimit, NewRun, RunFilter, RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use ironflow_worker::WorkerBuilder;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

/// Wall-clock budget for the whole flow.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a gauge may take to reach its expected value: past the 5 s
/// refresh period, so a missing gauge is not a timing artifact. A deadline,
/// not a fixed sleep, for slow CI executors.
const GAUGE_DEADLINE: Duration = Duration::from_secs(8);

/// Serializes the tests: they share the global Prometheus recorder.
static SERIAL: Mutex<()> = Mutex::const_new(());

/// A run no worker can pick yet: it stays `Pending` for the whole test.
fn future_run(workflow: &str) -> NewRun {
    NewRun {
        created_by: None,
        workflow_name: workflow.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: Some(Utc::now() + TimeDelta::hours(1)),
        idempotency_key: None,
        concurrency_key: None,
        priority: 0,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
    }
}

/// A due run in the single-slot group `repo:acme`.
fn grouped_run() -> NewRun {
    NewRun {
        created_by: None,
        workflow_name: "deploy".to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        idempotency_key: None,
        concurrency_key: None,
        priority: 0,
        concurrency_limits: vec![ConcurrencyLimit::new("repo:acme", 1)],
        max_cost_usd: None,
    }
}

/// Serve the real API router over TCP on top of `store`, in `Workers` mode.
///
/// Returns the base URL and a function rendering the global Prometheus
/// exposition.
async fn serve_api(store: Arc<InMemoryStore>) -> (String, impl Fn() -> String) {
    let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()))
        .with_execution_mode(ExecutionMode::Workers);
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-queue-depth".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(16);
    let state = AppState::new(
        store,
        Arc::new(engine),
        jwt_config,
        "test-worker-token".to_string(),
        event_sender,
    );
    let prometheus = state.prometheus_handle.clone();

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
    (format!("http://{addr}"), move || prometheus.render())
}

/// Start a single-slot worker polling the API at `api_url`.
fn spawn_worker(api_url: &str, worker_id: &str) -> JoinHandle<()> {
    let worker = WorkerBuilder::new(api_url, "test-worker-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .worker_id(worker_id)
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .build()
        .expect("build worker");
    spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("worker exited with error: {e:?}");
        }
    })
}

/// Poll `series` until it reads `expected` or [`GAUGE_DEADLINE`] elapses, and
/// return the last value read.
async fn wait_for_value(render: &impl Fn() -> String, series: &str, expected: f64) -> Option<f64> {
    let deadline = Instant::now() + GAUGE_DEADLINE;
    let mut value = None;
    while Instant::now() < deadline {
        value = gauge_value(&render(), series);
        if value == Some(expected) {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    value
}

/// The value of `series` (a bare metric name, or a name followed by its
/// labels) in a Prometheus text exposition, if present.
fn gauge_value(exposition: &str, series: &str) -> Option<f64> {
    exposition
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let (name, value) = line.rsplit_once(' ')?;
            (name == series)
                .then(|| value.trim().parse().ok())
                .flatten()
        })
}

#[tokio::test]
async fn api_mode_worker_publishes_pending_run_count_as_queue_depth() {
    let _serial = SERIAL.lock().await;
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        for _ in 0..3 {
            store
                .create_run(future_run("queued"))
                .await
                .expect("create pending run");
        }
        // A run that is no longer pending must not count.
        let running = store
            .create_run(future_run("queued"))
            .await
            .expect("create run")
            .into_run();
        store
            .update_run_status(running.id, RunStatus::Running)
            .await
            .expect("move run to running");

        let (api_url, render) = serve_api(store).await;
        let handle = spawn_worker(&api_url, "worker-queue-depth");

        let depth = wait_for_value(&render, WORKER_QUEUE_DEPTH, 3.0).await;
        handle.abort();

        assert_eq!(
            depth,
            Some(3.0),
            "expected {WORKER_QUEUE_DEPTH} 3, exposition was:\n{}",
            render()
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn api_mode_worker_publishes_runs_blocked_by_group_and_resets_vanished_groups() {
    let _serial = SERIAL.lock().await;
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        // The holder takes the only slot of the group: the next two runs are
        // due but no worker may start them.
        let holder = store
            .create_run(grouped_run())
            .await
            .expect("create holder run")
            .into_run();
        store
            .update_run_status(holder.id, RunStatus::Running)
            .await
            .expect("move holder to running");
        let mut blocked_ids = Vec::new();
        for _ in 0..2 {
            let run = store
                .create_run(grouped_run())
                .await
                .expect("create blocked run")
                .into_run();
            blocked_ids.push(run.id);
        }

        let (api_url, render) = serve_api(store.clone()).await;
        let handle = spawn_worker(&api_url, "worker-blocked-runs");

        let series = format!("{WORKER_QUEUE_BLOCKED_RUNS}{{group=\"repo:acme\"}}");
        let blocked = wait_for_value(&render, &series, 2.0).await;
        assert_eq!(
            blocked,
            Some(2.0),
            "expected {series} 2, exposition was:\n{}",
            render()
        );

        // The worker must not have started a run of the saturated group.
        let held = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        let running = held
            .items
            .iter()
            .filter(|r| r.status.state == RunStatus::Running)
            .count();
        assert_eq!(running, 1, "only the holder may run");

        // Once no run is held back any more, the API stops reporting the
        // group: the worker must drop its gauge to zero, not keep 2.
        for id in blocked_ids {
            store
                .update_run_status(id, RunStatus::Cancelled)
                .await
                .expect("cancel blocked run");
        }
        let reset = wait_for_value(&render, &series, 0.0).await;
        handle.abort();

        assert_eq!(
            reset,
            Some(0.0),
            "expected {series} 0 once the group drained, exposition was:\n{}",
            render()
        );
    })
    .await
    .expect("test timed out");
}
