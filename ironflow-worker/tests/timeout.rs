//! Timeout: handler blocks longer than run_timeout, the worker calls
//! fail_or_schedule_retry with a timeout message.

mod helpers;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_worker::WorkerBuilder;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

struct BlockingHandler;

impl WorkflowHandler for BlockingHandler {
    fn name(&self) -> &str {
        "blocking-workflow"
    }

    fn execute<'a>(
        &'a self,
        _ctx: &'a mut WorkflowContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
        Box::pin(async {
            sleep(Duration::from_secs(60)).await;
            Ok(())
        })
    }
}

#[tokio::test]
async fn run_timeout_triggers_fail_or_schedule_retry() {
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(
        run_id,
        "blocking-workflow",
        3,
    )]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(BlockingHandler)
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_secs(5))
        .lease_refresh_interval(Duration::from_millis(200))
        .run_timeout(Duration::from_millis(200))
        .build()
        .expect("build worker");

    if let Ok(Err(e)) = timeout(Duration::from_secs(5), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    let bodies = state.all_status_bodies();
    assert!(
        !bodies.is_empty(),
        "the worker never wrote the run status after timeout"
    );

    // Timeout is retryable (worker.rs passes retryable=true to fail_or_schedule_retry),
    // and max_retries=3, so the result should be Retrying.
    let has_retrying = bodies.iter().any(|b| {
        b.get("status")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s == "retrying")
    });
    assert!(
        has_retrying,
        "expected Retrying (timeout is retryable + max_retries=3), got: {bodies:?}"
    );

    // The error message should mention "timed out".
    let has_timeout_msg = bodies.iter().any(|b| {
        b.get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.contains("timed out"))
    });
    assert!(
        has_timeout_msg,
        "expected a timeout error message, got: {bodies:?}"
    );
}

#[tokio::test]
async fn run_timeout_without_retries_produces_failed() {
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(
        run_id,
        "blocking-workflow",
        0,
    )]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(BlockingHandler)
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_secs(5))
        .lease_refresh_interval(Duration::from_millis(200))
        .run_timeout(Duration::from_millis(200))
        .build()
        .expect("build worker");

    if let Ok(Err(e)) = timeout(Duration::from_secs(5), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    let bodies = state.all_status_bodies();
    assert!(
        !bodies.is_empty(),
        "the worker never wrote the run status after timeout"
    );

    let has_failed = bodies.iter().any(|b| {
        b.get("status")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s == "failed")
    });
    assert!(
        has_failed,
        "expected Failed (timeout + max_retries=0), got: {bodies:?}"
    );
}
