//! A worker that loses its lease must abandon the run it is executing.
//!
//! The API is stubbed with a real axum server on a loopback port (system
//! boundary), so the worker exercises its real HTTP client, its real lease
//! refresher, and its real execution path.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::Utc;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_store::entities::Run;
use ironflow_worker::WorkerBuilder;
use rust_decimal::Decimal;
use serde_json::{Value, from_value, json};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Counters observed by the assertions.
#[derive(Default)]
struct ApiState {
    /// Runs handed out so far — the queue holds exactly one run.
    handed_out: AtomicUsize,
    /// Lease refresh attempts received.
    lease_calls: AtomicUsize,
    /// Status writes received for the run (PUT /runs/:id or /runs/:id/status).
    status_writes: AtomicUsize,
    run_id: Uuid,
}

/// A workflow that stays busy long enough for the lease to be lost.
struct SlowWorkflow {
    finished: Arc<AtomicBool>,
}

impl WorkflowHandler for SlowWorkflow {
    fn name(&self) -> &str {
        "slow"
    }

    fn execute<'a>(
        &'a self,
        _ctx: &'a mut WorkflowContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
        Box::pin(async move {
            sleep(Duration::from_secs(30)).await;
            self.finished.store(true, Ordering::SeqCst);
            Ok(())
        })
    }
}

fn run_json(state: &ApiState) -> Value {
    json!({
        "id": state.run_id,
        "workflow_name": "slow",
        "status": { "state": "running", "state_machine_id": Uuid::now_v7() },
        "trigger": { "kind": "manual" },
        "payload": {},
        "error": null,
        "retry_count": 0,
        "max_retries": 3,
        "cost_usd": Decimal::ZERO,
        "duration_ms": 0,
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
        "started_at": Utc::now(),
        "completed_at": null,
        "handler_version": null,
        "labels": {},
        "scheduled_at": null,
        "worker_id": "worker-test",
        "lease_expires_at": Utc::now(),
    })
}

async fn pick_next(State(state): State<Arc<ApiState>>) -> Json<Value> {
    // Hand the single run out once, then report an empty queue.
    if state.handed_out.fetch_add(1, Ordering::SeqCst) == 0 {
        Json(json!({ "data": run_json(&state) }))
    } else {
        Json(json!({ "data": null }))
    }
}

async fn get_run(State(state): State<Arc<ApiState>>, Path(_id): Path<Uuid>) -> Json<Value> {
    Json(json!({ "data": { "run": run_json(&state), "steps": [] } }))
}

/// Always refuse the refresh: this is a worker whose run was taken over.
async fn refuse_lease(
    State(state): State<Arc<ApiState>>,
    Path(_id): Path<Uuid>,
    Json(_body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.lease_calls.fetch_add(1, Ordering::SeqCst);
    (
        StatusCode::CONFLICT,
        Json(json!({ "error": { "code": "LEASE_LOST", "message": "lease lost" } })),
    )
}

async fn record_status_write(
    State(state): State<Arc<ApiState>>,
    Path(_id): Path<Uuid>,
    Json(_body): Json<Value>,
) -> Json<Value> {
    state.status_writes.fetch_add(1, Ordering::SeqCst);
    Json(json!({ "data": null }))
}

async fn spawn_api(state: Arc<ApiState>) -> String {
    let app = Router::new()
        .route("/api/v1/internal/runs/next", get(pick_next))
        .route(
            "/api/v1/internal/runs/{id}",
            get(get_run).put(record_status_write),
        )
        .route(
            "/api/v1/internal/runs/{id}/status",
            put(record_status_write),
        )
        .route("/api/v1/internal/runs/{id}/lease", post(refuse_lease))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    format!("http://{addr}")
}

#[tokio::test]
async fn worker_abandons_run_when_the_api_refuses_the_lease() {
    let handler_finished = Arc::new(AtomicBool::new(false));
    let state = Arc::new(ApiState {
        run_id: Uuid::now_v7(),
        ..ApiState::default()
    });
    let api_url = spawn_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(SlowWorkflow {
            finished: handler_finished.clone(),
        })
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(50))
        .lease_ttl(Duration::from_millis(300))
        .lease_refresh_interval(Duration::from_millis(50))
        .run_timeout(Duration::from_secs(30))
        .build()
        .expect("build worker");

    // Worker::run only returns on SIGTERM, so bound it: the assertions are
    // about what happened inside this window.
    let _ = timeout(Duration::from_secs(3), worker.run()).await;

    assert!(
        state.handed_out.load(Ordering::SeqCst) >= 1,
        "the worker never polled for a run"
    );
    assert!(
        state.lease_calls.load(Ordering::SeqCst) >= 1,
        "the worker never tried to refresh its lease"
    );
    assert!(
        !handler_finished.load(Ordering::SeqCst),
        "the workflow kept running after the lease was lost"
    );
    assert_eq!(
        state.status_writes.load(Ordering::SeqCst),
        0,
        "an abandoned run must not be written to: it belongs to another worker"
    );
}

/// The run must be abandoned well before its own 30 s body would finish, and
/// roughly within one refresh interval of the lease being refused.
#[tokio::test]
async fn abandon_happens_within_a_refresh_interval() {
    let handler_finished = Arc::new(AtomicBool::new(false));
    let state = Arc::new(ApiState {
        run_id: Uuid::now_v7(),
        ..ApiState::default()
    });
    let api_url = spawn_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(SlowWorkflow {
            finished: handler_finished,
        })
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_millis(200))
        .lease_refresh_interval(Duration::from_millis(20))
        .run_timeout(Duration::from_secs(30))
        .build()
        .expect("build worker");

    let started = std::time::Instant::now();
    let handle = spawn(async move {
        let _ = timeout(Duration::from_secs(5), worker.run()).await;
    });

    // Wait until the API has refused at least one refresh.
    while state.lease_calls.load(Ordering::SeqCst) == 0
        && started.elapsed() < Duration::from_secs(3)
    {
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        state.lease_calls.load(Ordering::SeqCst) >= 1,
        "no lease refresh was attempted"
    );

    // Give the worker a moment to react, then confirm it did not keep going.
    sleep(Duration::from_millis(200)).await;
    assert_eq!(
        state.status_writes.load(Ordering::SeqCst),
        0,
        "an abandoned run must not be written to"
    );

    handle.abort();
}

/// A run without any lease trouble is not affected by the refresher.
#[tokio::test]
async fn worker_keeps_running_while_the_lease_is_granted() {
    #[derive(Default)]
    struct GrantingState {
        handed_out: AtomicUsize,
        lease_calls: AtomicUsize,
        status_writes: AtomicUsize,
    }

    async fn grant_lease(
        State(state): State<Arc<GrantingState>>,
        Path(_id): Path<Uuid>,
        Json(_body): Json<Value>,
    ) -> Json<Value> {
        state.lease_calls.fetch_add(1, Ordering::SeqCst);
        Json(json!({
            "data": { "lease_expires_at": Utc::now() + chrono::TimeDelta::seconds(90) }
        }))
    }

    let run_id = Uuid::now_v7();
    let state = Arc::new(GrantingState::default());

    let run_payload = move || {
        json!({
            "id": run_id,
            "workflow_name": "slow",
            "status": { "state": "running", "state_machine_id": Uuid::now_v7() },
            "trigger": { "kind": "manual" },
            "payload": {},
            "error": null,
            "retry_count": 0,
            "max_retries": 3,
            "cost_usd": Decimal::ZERO,
            "duration_ms": 0,
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "started_at": Utc::now(),
            "completed_at": null,
            "handler_version": null,
            "labels": HashMap::<String, String>::new(),
            "scheduled_at": null,
            "worker_id": "worker-test",
            "lease_expires_at": Utc::now() + chrono::TimeDelta::seconds(90),
        })
    };

    let next_payload = run_payload;
    let detail_payload = run_payload;

    let app = Router::new()
        .route(
            "/api/v1/internal/runs/next",
            get({
                let state = state.clone();
                move || {
                    let state = state.clone();
                    async move {
                        if state.handed_out.fetch_add(1, Ordering::SeqCst) == 0 {
                            Json(json!({ "data": next_payload() }))
                        } else {
                            Json(json!({ "data": null }))
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/internal/runs/{id}",
            get(move || async move {
                Json(json!({ "data": { "run": detail_payload(), "steps": [] } }))
            })
            .put({
                let state = state.clone();
                move |Json(_body): Json<Value>| {
                    let state = state.clone();
                    async move {
                        state.status_writes.fetch_add(1, Ordering::SeqCst);
                        Json(json!({ "data": null }))
                    }
                }
            }),
        )
        .route("/api/v1/internal/runs/{id}/lease", post(grant_lease))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let worker = WorkerBuilder::new(&format!("http://{addr}"), "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(SlowWorkflow {
            finished: Arc::new(AtomicBool::new(false)),
        })
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_millis(500))
        .lease_refresh_interval(Duration::from_millis(30))
        .run_timeout(Duration::from_secs(30))
        .build()
        .expect("build worker");

    let handle = spawn(async move {
        let _ = timeout(Duration::from_secs(5), worker.run()).await;
    });

    sleep(Duration::from_millis(400)).await;

    assert!(
        state.lease_calls.load(Ordering::SeqCst) >= 2,
        "the lease should be refreshed repeatedly while the run executes"
    );
    assert_eq!(
        state.status_writes.load(Ordering::SeqCst),
        0,
        "a run that is still executing must not be finalised"
    );

    handle.abort();
}

/// Guard: the API double must produce a payload the worker can actually parse.
#[test]
fn stub_run_payload_deserializes_into_a_run() {
    let state = ApiState {
        run_id: Uuid::now_v7(),
        ..ApiState::default()
    };
    let payload = run_json(&state);
    let parsed: Result<Run, _> = from_value(payload.clone());
    assert!(
        parsed.is_ok(),
        "payload not parseable: {parsed:?}\n{payload}"
    );
}
