//! Poison pill: after max_consecutive_panics failures for the same workflow,
//! the worker skips subsequent runs and marks them Failed directly via
//! update_run_status, without executing the handler.

mod helpers;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ironflow_core::error::OperationError;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_worker::WorkerBuilder;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

struct CountingFailHandler {
    executions: Arc<AtomicUsize>,
}

impl WorkflowHandler for CountingFailHandler {
    fn name(&self) -> &str {
        "poison-workflow"
    }

    fn execute<'a>(
        &'a self,
        _ctx: &'a mut WorkflowContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            // Small yield to let tokio schedule the watcher tasks that
            // report outcomes to the poison pill tracker.
            sleep(Duration::from_millis(5)).await;
            Err(EngineError::Operation(OperationError::Http {
                status: Some(503),
                message: "always fails".to_string(),
            }))
        })
    }
}

#[tokio::test]
async fn worker_skips_runs_after_consecutive_failures_reach_threshold() {
    let executions = Arc::new(AtomicUsize::new(0));

    // 6 runs for the same workflow, all with max_retries=0 so each failure
    // is terminal (Failed). max_consecutive_panics=3 means the poison pill
    // kicks in after 3 consecutive failures. The first 3 runs execute and
    // fail; subsequent runs should be skipped.
    let runs: Vec<_> = (0..6)
        .map(|_| make_run_json(Uuid::now_v7(), "poison-workflow", 0))
        .collect();

    let state = Arc::new(TestApiState::new(runs));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(CountingFailHandler {
            executions: executions.clone(),
        })
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_secs(5))
        .lease_refresh_interval(Duration::from_millis(200))
        .run_timeout(Duration::from_secs(10))
        .max_consecutive_panics(3)
        .panic_cooldown(Duration::from_secs(300))
        .build()
        .expect("build worker");

    if let Ok(Err(e)) = timeout(Duration::from_secs(10), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    let total_handed = state.handed_out.load(Ordering::SeqCst);
    let total_exec = executions.load(Ordering::SeqCst);

    // All 6 runs were polled from the queue.
    assert!(
        total_handed >= 4,
        "the worker should have polled at least 4 runs, got {total_handed}"
    );

    // The handler was NOT executed for all runs: the poison pill skipped some.
    assert!(
        total_exec < total_handed,
        "expected some runs to be skipped by poison pill: {total_exec} executed out of {total_handed} handed out"
    );

    // The threshold is 3, but concurrency=1 + poll_interval=20ms means the
    // poison pill check races with the 4th run being picked. In practice the
    // counter flips between 3 and 4 depending on scheduler timing; 6 is the
    // "definitely broken" value.
    assert!(
        total_exec <= 4,
        "expected at most 4 handler executions before poison pill, got {total_exec}"
    );

    // Skipped runs are marked Failed via update_run_status (the poison pill path
    // in the worker loop, NOT via finalize_run).
    let status_writes = state.status_writes.lock().unwrap();
    let poison_failed = status_writes
        .iter()
        .filter(|w| {
            w.body
                .get("status")
                .and_then(|s| s.as_str())
                .is_some_and(|s| s == "failed")
        })
        .count();
    assert!(
        poison_failed >= 1,
        "at least one run should be marked Failed via update_run_status (poison pill skip), got: {:?}",
        status_writes
            .iter()
            .map(|w| w.body.clone())
            .collect::<Vec<_>>()
    );
}
