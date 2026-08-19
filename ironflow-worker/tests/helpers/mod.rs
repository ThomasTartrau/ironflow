//! Shared test stub for the ironflow-worker integration tests.
//!
//! Provides a configurable axum API double that records every request the worker
//! makes. Each test builds a [`TestApiState`] with the behavior it needs
//! (refuse lease, return errors, inject N runs, ...) and spawns it on a random
//! loopback port.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{TimeDelta, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::spawn;
use uuid::Uuid;

/// A status update received by the stub (via PUT /runs/:id or PUT /runs/:id/status).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RecordedStatusWrite {
    pub run_id: Uuid,
    pub body: Value,
}

/// Shared state between the test and the stub API.
pub struct TestApiState {
    /// Runs to hand out, consumed in FIFO order.
    runs: Mutex<VecDeque<Value>>,
    /// How many runs have been handed out.
    pub handed_out: AtomicUsize,
    /// Lease refresh attempts received.
    pub lease_calls: AtomicUsize,
    /// Status writes via PUT /runs/:id (update_run / update_run_returning).
    pub run_updates: Mutex<Vec<RecordedStatusWrite>>,
    /// Status writes via PUT /runs/:id/status (update_run_status).
    pub status_writes: Mutex<Vec<RecordedStatusWrite>>,
    /// Whether the lease should be refused (409 LEASE_LOST).
    pub refuse_lease: AtomicBool,
    /// Runs indexed by id, populated when handed out via pick_next.
    runs_by_id: Mutex<HashMap<Uuid, Value>>,
}

impl TestApiState {
    pub fn new(runs: Vec<Value>) -> Self {
        Self {
            runs: Mutex::new(VecDeque::from(runs)),
            handed_out: AtomicUsize::new(0),
            lease_calls: AtomicUsize::new(0),
            run_updates: Mutex::new(Vec::new()),
            status_writes: Mutex::new(Vec::new()),
            refuse_lease: AtomicBool::new(false),
            runs_by_id: Mutex::new(HashMap::new()),
        }
    }

    #[allow(dead_code)]
    pub fn total_status_writes(&self) -> usize {
        self.run_updates.lock().unwrap().len() + self.status_writes.lock().unwrap().len()
    }

    #[allow(dead_code)]
    pub fn all_status_bodies(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self
            .run_updates
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.body.clone())
            .collect();
        out.extend(
            self.status_writes
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.body.clone()),
        );
        out
    }
}

/// Build a run JSON payload suitable for the stub API.
pub fn make_run_json(run_id: Uuid, workflow_name: &str, max_retries: u32) -> Value {
    json!({
        "id": run_id,
        "workflow_name": workflow_name,
        "status": { "state": "running", "state_machine_id": Uuid::now_v7() },
        "trigger": { "kind": "manual" },
        "payload": {},
        "error": null,
        "retry_count": 0,
        "max_retries": max_retries,
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

async fn pick_next(State(state): State<Arc<TestApiState>>) -> (StatusCode, Json<Value>) {
    let mut runs = state.runs.lock().unwrap();
    if let Some(run) = runs.pop_front() {
        drop(runs);
        state.handed_out.fetch_add(1, Ordering::SeqCst);
        let id = run["id"]
            .as_str()
            .expect("run must have an id")
            .parse::<Uuid>()
            .expect("run id must be a valid UUID");
        state.runs_by_id.lock().unwrap().insert(id, run.clone());
        (StatusCode::OK, Json(json!({ "data": run })))
    } else {
        (StatusCode::OK, Json(json!({ "data": null })))
    }
}

async fn get_run(State(state): State<Arc<TestApiState>>, Path(id): Path<Uuid>) -> Json<Value> {
    let run = state
        .runs_by_id
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .expect("get_run called for an id that was never handed out");
    Json(json!({ "data": { "run": run, "steps": [] } }))
}

async fn update_run(
    State(state): State<Arc<TestApiState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state
        .run_updates
        .lock()
        .unwrap()
        .push(RecordedStatusWrite { run_id: id, body });
    Json(json!({ "data": null }))
}

async fn update_run_status(
    State(state): State<Arc<TestApiState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state
        .status_writes
        .lock()
        .unwrap()
        .push(RecordedStatusWrite { run_id: id, body });
    Json(json!({ "data": null }))
}

async fn renew_lease(
    State(state): State<Arc<TestApiState>>,
    Path(_id): Path<Uuid>,
    Json(_body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.lease_calls.fetch_add(1, Ordering::SeqCst);
    if state.refuse_lease.load(Ordering::SeqCst) {
        (
            StatusCode::CONFLICT,
            Json(json!({ "error": { "code": "LEASE_LOST", "message": "lease lost" } })),
        )
    } else {
        (
            StatusCode::OK,
            Json(json!({
                "data": { "lease_expires_at": Utc::now() + TimeDelta::seconds(90) }
            })),
        )
    }
}

async fn create_step(Json(body): Json<Value>) -> Json<Value> {
    let step_id = Uuid::now_v7();
    let run_id = body["run_id"]
        .as_str()
        .expect("worker must send run_id")
        .parse::<Uuid>()
        .expect("run_id must be a valid UUID");
    let name = body["name"].as_str().expect("worker must send step name");
    Json(json!({
        "data": {
            "id": step_id,
            "run_id": run_id,
            "name": name,
            "kind": body.get("kind").cloned().unwrap_or(json!("shell")),
            "position": body.get("position").cloned().unwrap_or(json!(0)),
            "status": { "state": "pending", "state_machine_id": Uuid::now_v7() },
            "attempt": 1,
            "input": null,
            "output": null,
            "error": null,
            "duration_ms": 0,
            "cost_usd": Decimal::ZERO,
            "input_tokens": null,
            "output_tokens": null,
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "started_at": null,
            "completed_at": null,
            "debug_messages": null,
            "is_error_handler": false,
        }
    }))
}

async fn update_step(Path(_id): Path<Uuid>, Json(_body): Json<Value>) -> Json<Value> {
    Json(json!({ "data": null }))
}

async fn push_logs(Path(_id): Path<Uuid>, Json(_body): Json<Value>) -> Json<Value> {
    Json(json!({ "data": null }))
}

async fn create_step_deps(Json(_body): Json<Value>) -> Json<Value> {
    Json(json!({ "data": null }))
}

async fn list_artifacts(Path(_id): Path<Uuid>) -> Json<Value> {
    Json(json!({ "data": [] }))
}

/// Spawn a test API on a random loopback port and return its URL.
pub async fn spawn_test_api(state: Arc<TestApiState>) -> String {
    let app = Router::new()
        .route("/api/v1/internal/runs/next", get(pick_next))
        .route("/api/v1/internal/runs/{id}", get(get_run).put(update_run))
        .route("/api/v1/internal/runs/{id}/status", put(update_run_status))
        .route("/api/v1/internal/runs/{id}/lease", post(renew_lease))
        .route("/api/v1/internal/runs/{id}/logs", post(push_logs))
        .route("/api/v1/internal/runs/{id}/artifacts", get(list_artifacts))
        .route("/api/v1/internal/steps", post(create_step))
        .route("/api/v1/internal/steps/{id}", put(update_step))
        .route("/api/v1/internal/step-dependencies", post(create_step_deps))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    format!("http://{addr}")
}
