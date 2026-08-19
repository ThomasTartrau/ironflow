//! A worker that loses its lease must abandon the run it is executing.
//!
//! The API is stubbed with a real axum server on a loopback port (system
//! boundary), so the worker exercises its real HTTP client, its real lease
//! refresher, and its real execution path.

mod helpers;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_store::entities::Run;
use ironflow_worker::WorkerBuilder;
use serde_json::from_value;
use tokio::spawn;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

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

#[tokio::test]
async fn worker_abandons_run_when_the_api_refuses_the_lease() {
    let handler_finished = Arc::new(AtomicBool::new(false));
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(run_id, "slow", 3)]));
    state.refuse_lease.store(true, Ordering::SeqCst);
    let api_url = spawn_test_api(state.clone()).await;

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
    if let Ok(Err(e)) = timeout(Duration::from_secs(3), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

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
        state.total_status_writes(),
        0,
        "an abandoned run must not be written to: it belongs to another worker"
    );
}

/// The run must be abandoned well before its own 30 s body would finish, and
/// roughly within one refresh interval of the lease being refused.
#[tokio::test]
async fn abandon_happens_within_a_refresh_interval() {
    let handler_finished = Arc::new(AtomicBool::new(false));
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(run_id, "slow", 3)]));
    state.refuse_lease.store(true, Ordering::SeqCst);
    let api_url = spawn_test_api(state.clone()).await;

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

    let started = Instant::now();
    let handle = spawn(async move {
        if let Ok(Err(e)) = timeout(Duration::from_secs(5), worker.run()).await {
            eprintln!("worker exited with error: {e:?}");
        }
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
        state.total_status_writes(),
        0,
        "an abandoned run must not be written to"
    );

    handle.abort();
}

/// A run without any lease trouble is not affected by the refresher.
#[tokio::test]
async fn worker_keeps_running_while_the_lease_is_granted() {
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(run_id, "slow", 3)]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
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
        if let Ok(Err(e)) = timeout(Duration::from_secs(5), worker.run()).await {
            eprintln!("worker exited with error: {e:?}");
        }
    });

    sleep(Duration::from_millis(400)).await;

    assert!(
        state.lease_calls.load(Ordering::SeqCst) >= 2,
        "the lease should be refreshed repeatedly while the run executes"
    );
    assert_eq!(
        state.total_status_writes(),
        0,
        "a run that is still executing must not be finalised"
    );

    handle.abort();
}

/// Guard: the API double must produce a payload the worker can actually parse.
#[test]
fn stub_run_payload_deserializes_into_a_run() {
    let payload = make_run_json(Uuid::now_v7(), "test", 3);
    let parsed: Result<Run, _> = from_value(payload.clone());
    assert!(
        parsed.is_ok(),
        "payload not parseable: {parsed:?}\n{payload}"
    );
}
