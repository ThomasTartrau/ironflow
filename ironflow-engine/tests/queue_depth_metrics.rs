//! Integration tests for the `ironflow_worker_queue_depth` Prometheus gauge.
//!
//! Verifies that pending runs are accurately reflected in the queue depth
//! metric, and that the metric drops after runs are executed.

#![cfg(feature = "prometheus")]

use std::collections::HashMap;
use std::sync::Arc;

use ironflow_core::metric_names::WORKER_QUEUE_DEPTH;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, RunFilter, RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use metrics::gauge;
use metrics_exporter_prometheus::PrometheusBuilder;
use serde_json::json;

struct EchoWorkflow;

impl WorkflowHandler for EchoWorkflow {
    fn name(&self) -> &str {
        "queue-depth-test"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("echo", ShellConfig::new("echo ok")).await?;
            Ok(())
        })
    }
}

fn create_engine() -> (Engine, Arc<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let inner = ClaudeCodeProvider::new();
    let provider = Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ));
    let engine = Engine::new(store.clone(), provider);
    (engine, store)
}

#[tokio::test]
async fn queue_depth_reflects_pending_runs() {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install recorder");

    let (mut engine, store) = create_engine();
    engine.register(EchoWorkflow).unwrap();

    // Initially: no pending runs -> queue depth = 0
    let stats = store
        .get_stats(RunFilter {
            status: Some(RunStatus::Pending),
            ..RunFilter::default()
        })
        .await
        .unwrap();
    gauge!(WORKER_QUEUE_DEPTH).set(stats.total_runs as f64);

    let output = handle.render();
    assert!(
        output.contains(WORKER_QUEUE_DEPTH),
        "output must contain {WORKER_QUEUE_DEPTH}"
    );
    assert!(
        output.contains(&format!("{WORKER_QUEUE_DEPTH} 0")),
        "queue depth must be 0 when no runs are pending"
    );

    // Create 3 pending runs (create_run starts them as Pending)
    for _ in 0..3 {
        store
            .create_run(NewRun {
                workflow_name: "queue-depth-test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();
    }

    let stats = store
        .get_stats(RunFilter {
            status: Some(RunStatus::Pending),
            ..RunFilter::default()
        })
        .await
        .unwrap();
    gauge!(WORKER_QUEUE_DEPTH).set(stats.total_runs as f64);

    let output = handle.render();
    assert!(
        output.contains(&format!("{WORKER_QUEUE_DEPTH} 3")),
        "queue depth must be 3 with 3 pending runs, got: {output}"
    );

    // Execute all 3 runs (transitions them out of Pending)
    for _ in 0..3 {
        let run = store.pick_next_pending(None).await.unwrap();
        assert!(run.is_some(), "expected a pending run to pick");
        let run = run.unwrap();
        engine.execute_handler_run(run.id).await.unwrap();
    }

    let stats = store
        .get_stats(RunFilter {
            status: Some(RunStatus::Pending),
            ..RunFilter::default()
        })
        .await
        .unwrap();
    gauge!(WORKER_QUEUE_DEPTH).set(stats.total_runs as f64);

    let output = handle.render();
    assert!(
        output.contains(&format!("{WORKER_QUEUE_DEPTH} 0")),
        "queue depth must return to 0 after all runs are executed, got: {output}"
    );
}
