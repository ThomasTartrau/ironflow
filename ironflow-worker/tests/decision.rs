//! `ctx.decision(...)` inside a workflow executed by the worker.
//!
//! The decision provider is a [`RecordReplayDecisionProvider`] replaying the real
//! Jev response captured in `tests/fixtures/decisions/triage-output.json` (same
//! capture as `ironflow-engine/tests/fixtures/decisions/`). No mocks.

mod helpers;

use std::env::temp_dir;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use ironflow_core::decision::DecisionOutput;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay_decision::{
    RecordReplayDecisionProvider, hash_request,
};
use ironflow_engine::config::{DecisionConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::decision::{DecisionAnswers, DecisionChoice};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_worker::{Worker, WorkerBuilder};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::spawn;
use tokio::time::sleep;
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

const WORKFLOW: &str = "triage";

/// Options of the `department` question.
#[derive(Debug, Clone, Copy, PartialEq, DecisionChoice)]
enum Department {
    Billing,
    Technical,
    Sales,
}

/// The questions of the recorded request, one per field.
#[derive(Debug, DecisionAnswers)]
#[allow(dead_code)]
struct Triage {
    #[noul("Does this convey urgency?")]
    is_urgent: f64,
    #[choice("Which team?")]
    department: Department,
    #[score("How frustrated?", levels = ["Calm", "Frustrated", "Very angry"])]
    frustration: f64,
}

fn build_config() -> DecisionConfig<Triage> {
    DecisionConfig::new("Help! My payouts have been failing for 3 days.").answers::<Triage>()
}

/// The real Jev response recorded for [`build_config`]: `department` is "billing".
fn real_triage_output() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/decisions/triage-output.json"
    );
    let json = fs::read_to_string(path).expect("recorded triage fixture must exist");
    serde_json::from_str(&json).expect("recorded fixture is valid JSON")
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

/// An empty fixtures directory: a replay provider on it fails on any call.
fn empty_fixtures() -> (String, TempDir) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = temp_dir().join(format!(
        "ironflow-worker-decision-{}-{}",
        process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    (dir.to_string_lossy().into_owned(), TempDir(dir))
}

/// A fixtures directory holding the recorded answer for [`build_config`].
fn recorded_fixtures() -> (String, TempDir) {
    let (dir, guard) = empty_fixtures();
    let request = build_config().to_request();
    let fixture = json!({ "request": request, "output": real_triage_output() });
    let path = PathBuf::from(&dir).join(format!("{}.json", hash_request(&request)));
    fs::write(path, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
    (dir, guard)
}

/// One decision step, then a shell step that only runs when the decision
/// returned "billing".
struct TriageWorkflow;

impl WorkflowHandler for TriageWorkflow {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let triage = ctx.decision("triage", build_config()).await?;
            if triage.department == Department::Billing {
                ctx.shell("route", ShellConfig::new("echo routed to billing"))
                    .await?;
            }
            Ok(())
        })
    }
}

fn builder(api_url: &str) -> WorkerBuilder {
    WorkerBuilder::new(api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(TriageWorkflow)
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_millis(500))
        .lease_refresh_interval(Duration::from_millis(50))
        .run_timeout(Duration::from_secs(10))
}

fn terminal_status(state: &TestApiState) -> Option<(String, Value)> {
    state.all_status_bodies().into_iter().find_map(|body| {
        let status = body.get("status")?.as_str()?.to_string();
        matches!(status.as_str(), "completed" | "failed").then_some((status, body))
    })
}

/// Run the worker until it writes a terminal status for the run, then stop it.
async fn run_until_terminal(worker: Worker, state: &TestApiState) -> (String, Value) {
    let handle = spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("worker exited with error: {e:?}");
        }
    });

    // Generous deadline rather than a fixed sleep: the worker task may be
    // scheduled late on an oversubscribed CI executor.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut outcome = terminal_status(state);
    while outcome.is_none() && Instant::now() < deadline {
        sleep(Duration::from_millis(20)).await;
        outcome = terminal_status(state);
    }
    handle.abort();

    outcome.unwrap_or_else(|| {
        panic!(
            "the worker never wrote a terminal status, got: {:?}",
            state.all_status_bodies()
        )
    })
}

fn created_step_names(state: &TestApiState) -> Vec<String> {
    state
        .created_steps
        .lock()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn decision_step_uses_the_worker_decision_provider() {
    let (dir, _guard) = recorded_fixtures();
    let state = Arc::new(TestApiState::new(vec![make_run_json(
        Uuid::now_v7(),
        WORKFLOW,
        0,
    )]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = builder(&api_url)
        .decision_provider(Arc::new(RecordReplayDecisionProvider::replay(&dir)))
        .build()
        .expect("build worker");

    let (status, body) = run_until_terminal(worker, &state).await;
    assert_eq!(status, "completed", "run did not complete: {body}");

    let created = state.created_steps.lock().unwrap().clone();
    let decision = created
        .iter()
        .find(|s| s["name"] == "triage")
        .expect("the decision step was never created");
    assert_eq!(decision["kind"], "decision");

    // The recorded answers were persisted on the decision step.
    let stored = state
        .step_updates
        .lock()
        .unwrap()
        .iter()
        .find_map(|u| u.get("output").filter(|o| !o.is_null()).cloned())
        .expect("the decision output was never persisted");
    let stored: DecisionOutput = serde_json::from_value(stored).unwrap();
    assert_eq!(stored.choice("department").unwrap().choice, "billing");

    // The routing step ran, proving the handler read "billing" from the provider.
    assert!(created_step_names(&state).contains(&"route".to_string()));
}

#[tokio::test]
async fn decision_step_without_provider_fails_the_run() {
    let state = Arc::new(TestApiState::new(vec![make_run_json(
        Uuid::now_v7(),
        WORKFLOW,
        0,
    )]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = builder(&api_url).build().expect("build worker");

    let (status, body) = run_until_terminal(worker, &state).await;
    assert_eq!(
        status, "failed",
        "run should fail without a provider: {body}"
    );
    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("decision step 'triage' requires a decision provider"),
        "expected the NoDecisionProvider error, got: {body}"
    );
    assert!(!created_step_names(&state).contains(&"route".to_string()));
}

#[tokio::test]
async fn completed_decision_step_is_replayed_without_calling_the_provider() {
    // Empty fixtures: the provider fails if the worker calls it.
    let (dir, _guard) = empty_fixtures();
    let run_id = Uuid::now_v7();

    // A retried run (attempt 2) whose decision step already completed in this
    // attempt: the worker loads it from the store and replays it.
    let mut run = make_run_json(run_id, WORKFLOW, 1);
    run["retry_count"] = json!(1);
    let decision_step = json!({
        "id": Uuid::now_v7(),
        "trace_id": Uuid::now_v7(),
        "run_id": run_id,
        "name": "triage",
        "kind": "decision",
        "position": 0,
        "status": { "state": "completed", "state_machine_id": Uuid::now_v7() },
        "attempt": 2,
        "input": null,
        "output": real_triage_output(),
        "error": null,
        "duration_ms": 12,
        "cost_usd": Decimal::ZERO,
        "input_tokens": 353,
        "output_tokens": 73,
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
        "started_at": Utc::now(),
        "completed_at": Utc::now(),
        "debug_messages": null,
        "is_error_handler": false,
    });
    let state = Arc::new(TestApiState::new(vec![run]).with_steps(vec![decision_step]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = builder(&api_url)
        .decision_provider(Arc::new(RecordReplayDecisionProvider::replay(&dir)))
        .build()
        .expect("build worker");

    let (status, body) = run_until_terminal(worker, &state).await;
    assert_eq!(status, "completed", "replayed run did not complete: {body}");

    // Only the routing step is new: the decision came from the store.
    assert_eq!(created_step_names(&state), vec!["route".to_string()]);
}
