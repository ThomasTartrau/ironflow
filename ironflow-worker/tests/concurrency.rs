//! Concurrency: 2 runs in the queue with concurrency(2), both execute in
//! parallel (their handlers overlap in time).

mod helpers;

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_worker::WorkerBuilder;
use tokio::sync::Barrier;
use tokio::time::timeout;
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

struct BarrierHandler {
    barrier: Arc<Barrier>,
    started: Arc<AtomicUsize>,
}

impl WorkflowHandler for BarrierHandler {
    fn name(&self) -> &str {
        "barrier-workflow"
    }

    fn execute<'a>(
        &'a self,
        _ctx: &'a mut WorkflowContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
        self.started.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            // Both handlers must reach this barrier to proceed.
            // If concurrency < 2, the second handler never starts and
            // the barrier times out.
            self.barrier.wait().await;
            Ok(())
        })
    }
}

#[tokio::test]
async fn two_runs_execute_in_parallel_with_concurrency_two() {
    let started = Arc::new(AtomicUsize::new(0));
    // Barrier for 2 parties: both handlers must reach it to proceed.
    let barrier = Arc::new(Barrier::new(2));

    let runs = vec![
        make_run_json(Uuid::now_v7(), "barrier-workflow", 0),
        make_run_json(Uuid::now_v7(), "barrier-workflow", 0),
    ];

    let state = Arc::new(TestApiState::new(runs));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(BarrierHandler {
            barrier,
            started: started.clone(),
        })
        .worker_id("worker-test")
        .concurrency(2)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_secs(5))
        .lease_refresh_interval(Duration::from_millis(200))
        .run_timeout(Duration::from_secs(10))
        .build()
        .expect("build worker");

    if let Ok(Err(e)) = timeout(Duration::from_secs(5), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    assert_eq!(
        state.handed_out.load(Ordering::SeqCst),
        2,
        "the worker should have polled both runs"
    );

    // Both handlers started (the barrier would deadlock if only one ran).
    assert_eq!(
        started.load(Ordering::SeqCst),
        2,
        "both handlers should have started concurrently"
    );

    // Both runs should have completed (Barrier passed => Ok(()) => Completed).
    let updates = state.run_updates.lock().unwrap();
    let completed: Vec<_> = updates
        .iter()
        .filter(|w| {
            w.body
                .get("status")
                .and_then(|s| s.as_str())
                .is_some_and(|s| s == "completed")
        })
        .collect();
    assert_eq!(
        completed.len(),
        2,
        "both runs should be marked Completed, got: {:?}",
        updates.iter().map(|w| w.body.clone()).collect::<Vec<_>>()
    );

    let distinct_ids: HashSet<_> = completed.iter().map(|w| w.run_id).collect();
    assert_eq!(
        distinct_ids.len(),
        2,
        "completed writes should be for 2 distinct run_ids"
    );
}
