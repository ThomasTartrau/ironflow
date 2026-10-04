//! A worker polling the API publishes `ironflow_worker_queue_depth`: the number
//! of `Pending` runs, read from the API server that owns the store.
//!
//! Everything is real: the router built by `create_router` served over TCP, an
//! `InMemoryStore` and a `Worker` polling that API. The API and the worker share
//! this process, hence one global Prometheus recorder: the one `AppState`
//! installs, read back through its handle.

#![cfg(feature = "prometheus")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::serve;
use chrono::{TimeDelta, Utc};
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::metric_names::WORKER_QUEUE_DEPTH;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::notify::Event;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use ironflow_worker::WorkerBuilder;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::time::{sleep, timeout};

/// Wall-clock budget for the whole flow.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

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
        max_cost_usd: None,
    }
}

/// The value of `metric` in a Prometheus text exposition, if present.
fn gauge_value(exposition: &str, metric: &str) -> Option<f64> {
    exposition
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let (name, value) = line.split_once(' ')?;
            (name == metric)
                .then(|| value.trim().parse().ok())
                .flatten()
        })
}

#[tokio::test]
async fn api_mode_worker_publishes_pending_run_count_as_queue_depth() {
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
            store.clone(),
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

        let worker = WorkerBuilder::new(&format!("http://{addr}"), "test-worker-token")
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .worker_id("worker-queue-depth")
            .concurrency(1)
            .poll_interval(Duration::from_millis(20))
            .build()
            .expect("build worker");
        let handle = spawn(async move {
            if let Err(e) = worker.run().await {
                eprintln!("worker exited with error: {e:?}");
            }
        });

        // Past the 5 s refresh period, so a missing gauge is not a timing
        // artifact. A deadline, not a fixed sleep, for slow CI executors.
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut depth = None;
        while Instant::now() < deadline {
            depth = gauge_value(&prometheus.render(), WORKER_QUEUE_DEPTH);
            if depth == Some(3.0) {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        handle.abort();

        assert_eq!(
            depth,
            Some(3.0),
            "expected {WORKER_QUEUE_DEPTH} 3, exposition was:\n{}",
            prometheus.render()
        );
    })
    .await
    .expect("test timed out");
}
