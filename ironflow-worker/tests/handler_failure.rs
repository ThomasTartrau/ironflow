//! Handler failure: handler returns Err, the worker reports Failed or Retrying
//! depending on error retryability and max_retries.

mod helpers;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ironflow_core::error::OperationError;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_worker::WorkerBuilder;
use tokio::time::timeout;
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

/// Non-retryable failure (InvalidWorkflow is never retried).
struct NonRetryableFailHandler;

impl WorkflowHandler for NonRetryableFailHandler {
    fn name(&self) -> &str {
        "fail-workflow"
    }

    fn execute<'a>(
        &'a self,
        _ctx: &'a mut WorkflowContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
        Box::pin(async {
            Err(EngineError::InvalidWorkflow(
                "intentional test failure".to_string(),
            ))
        })
    }
}

/// Retryable failure (HTTP 503 is retried when retries remain).
struct RetryableFailHandler;

impl WorkflowHandler for RetryableFailHandler {
    fn name(&self) -> &str {
        "retry-workflow"
    }

    fn execute<'a>(
        &'a self,
        _ctx: &'a mut WorkflowContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
        Box::pin(async {
            Err(EngineError::Operation(OperationError::Http {
                status: Some(503),
                message: "service unavailable".to_string(),
            }))
        })
    }
}

#[tokio::test]
async fn non_retryable_error_produces_failed() {
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(
        run_id,
        "fail-workflow",
        3,
    )]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(NonRetryableFailHandler)
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_millis(500))
        .lease_refresh_interval(Duration::from_millis(50))
        .run_timeout(Duration::from_secs(10))
        .build()
        .expect("build worker");

    if let Ok(Err(e)) = timeout(Duration::from_secs(3), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    let bodies = state.all_status_bodies();
    assert!(
        !bodies.is_empty(),
        "the worker never wrote the run status after failure"
    );

    let has_failed = bodies.iter().any(|b| {
        b.get("status")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s == "failed")
    });
    assert!(
        has_failed,
        "expected Failed (non-retryable error, even with max_retries=3), got: {bodies:?}"
    );
}

#[tokio::test]
async fn retryable_error_with_retries_produces_retrying() {
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(
        run_id,
        "retry-workflow",
        3,
    )]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(RetryableFailHandler)
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_millis(500))
        .lease_refresh_interval(Duration::from_millis(50))
        .run_timeout(Duration::from_secs(10))
        .build()
        .expect("build worker");

    if let Ok(Err(e)) = timeout(Duration::from_secs(3), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    let bodies = state.all_status_bodies();
    assert!(
        !bodies.is_empty(),
        "the worker never wrote the run status after failure"
    );

    let has_retrying = bodies.iter().any(|b| {
        b.get("status")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s == "retrying")
    });
    assert!(
        has_retrying,
        "expected Retrying (retryable error + max_retries=3), got: {bodies:?}"
    );
}

#[tokio::test]
async fn retryable_error_without_retries_produces_failed() {
    let run_id = Uuid::now_v7();
    let state = Arc::new(TestApiState::new(vec![make_run_json(
        run_id,
        "retry-workflow",
        0,
    )]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(RetryableFailHandler)
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_millis(500))
        .lease_refresh_interval(Duration::from_millis(50))
        .run_timeout(Duration::from_secs(10))
        .build()
        .expect("build worker");

    if let Ok(Err(e)) = timeout(Duration::from_secs(3), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    let bodies = state.all_status_bodies();
    assert!(
        !bodies.is_empty(),
        "the worker never wrote the run status after failure"
    );

    let has_failed = bodies.iter().any(|b| {
        b.get("status")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s == "failed")
    });
    assert!(
        has_failed,
        "expected Failed (retryable error but max_retries=0), got: {bodies:?}"
    );
}
