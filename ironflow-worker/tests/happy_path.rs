//! Happy path: handler returns Ok(()), the worker reports Completed.

mod helpers;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_worker::WorkerBuilder;
use tokio::time::timeout;
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

struct OkHandler;

impl WorkflowHandler for OkHandler {
    fn name(&self) -> &str {
        "ok-workflow"
    }

    fn execute<'a>(
        &'a self,
        _ctx: &'a mut WorkflowContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn handler_ok_produces_completed_status() {
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(
        run_id,
        "ok-workflow",
        0,
    )]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(OkHandler)
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_millis(500))
        .lease_refresh_interval(Duration::from_millis(50))
        .run_timeout(Duration::from_secs(10))
        .build()
        .expect("build worker");

    // Worker::run loops until SIGTERM; the timeout bounds the test window.
    if let Ok(Err(e)) = timeout(Duration::from_secs(3), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    assert!(
        state.handed_out.load(Ordering::SeqCst) >= 1,
        "the worker never polled for a run"
    );

    // finalize_run calls update_run (PUT /runs/:id) with status Completed.
    let updates = state.run_updates.lock().unwrap();
    assert!(!updates.is_empty(), "the worker never wrote the run status");

    let completed = updates.iter().any(|w| {
        w.body
            .get("status")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s == "completed")
    });
    assert!(
        completed,
        "expected a Completed status write, got: {:?}",
        updates.iter().map(|w| w.body.clone()).collect::<Vec<_>>()
    );
}
