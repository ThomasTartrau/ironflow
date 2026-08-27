//! Integration tests for Prometheus metrics instrumentation.
//!
//! These tests verify that the engine emits the correct Prometheus metrics
//! when the `prometheus` feature is enabled.

#![cfg(feature = "prometheus")]

use std::sync::Arc;

use ironflow_core::metric_names::{
    RUN_COST_USD, RUN_DURATION_SECONDS, RUNS_TOTAL, SHELL_DURATION_SECONDS, SHELL_TOTAL,
    STEP_DURATION_SECONDS, STEPS_TOTAL,
};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::TriggerKind;
use metrics_exporter_prometheus::PrometheusBuilder;
use serde_json::json;

struct MetricsTestWorkflow;

impl WorkflowHandler for MetricsTestWorkflow {
    fn name(&self) -> &str {
        "metrics-test"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("echo-step", ShellConfig::new("echo hello"))
                .await?;
            Ok(())
        })
    }
}

fn create_test_engine() -> Engine {
    let store = Arc::new(InMemoryStore::new());
    let inner = ClaudeCodeProvider::new();
    let provider = Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ));
    Engine::new(store, provider)
}

#[tokio::test]
async fn run_completion_emits_metrics() {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install recorder");

    let mut engine = create_test_engine();
    engine.register(MetricsTestWorkflow).unwrap();

    let run = engine
        .run_handler("metrics-test", TriggerKind::Manual, json!({}))
        .await
        .unwrap()
        .run;

    assert_eq!(
        run.status.state,
        ironflow_store::models::RunStatus::Completed
    );

    let output = handle.render();

    // Run counter must contain the workflow name and completed status
    assert!(
        output.contains(RUNS_TOTAL),
        "metrics output must contain {RUNS_TOTAL}"
    );
    assert!(
        output.contains("workflow=\"metrics-test\""),
        "metrics output must contain workflow label"
    );
    assert!(
        output.contains("status=\"Completed\""),
        "metrics output must contain Completed status"
    );

    // Duration histogram must be present
    assert!(
        output.contains(RUN_DURATION_SECONDS),
        "metrics output must contain {RUN_DURATION_SECONDS}"
    );

    // Cost histogram must be present
    assert!(
        output.contains(RUN_COST_USD),
        "metrics output must contain {RUN_COST_USD}"
    );

    // Step-level metrics
    assert!(
        output.contains(STEPS_TOTAL),
        "metrics output must contain {STEPS_TOTAL}"
    );
    assert!(
        output.contains(STEP_DURATION_SECONDS),
        "metrics output must contain {STEP_DURATION_SECONDS}"
    );

    // Shell-specific metrics
    assert!(
        output.contains(SHELL_TOTAL),
        "metrics output must contain {SHELL_TOTAL}"
    );
    assert!(
        output.contains(SHELL_DURATION_SECONDS),
        "metrics output must contain {SHELL_DURATION_SECONDS}"
    );
}
