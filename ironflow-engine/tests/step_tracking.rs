//! Integration tests for granular step tracking: deterministic trace IDs,
//! persist_progress snapshots, and StepResult accumulation.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, StepStatus, TriggerKind, step_trace_id};
use ironflow_store::store::Store;

fn provider() -> Arc<dyn ironflow_core::provider::AgentProvider> {
    let inner = ClaudeCodeProvider::new();
    Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ))
}

async fn run_ctx() -> (WorkflowContext, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let run = store
        .create_run(NewRun {
            workflow_name: "test".to_string(),
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
        .unwrap()
        .into_run();

    let ctx = WorkflowContext::new(run.id, store.clone(), provider());
    (ctx, store)
}

#[tokio::test]
async fn persist_progress_updates_run_after_step() {
    let (mut ctx, store) = run_ctx().await;
    let run_id = ctx.run_id();

    // Before any step, run has zero cost/duration
    let run_before = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run_before.duration_ms, 0);

    // Execute a shell step
    ctx.shell("wait_step", ShellConfig::new("sleep 0.01"))
        .await
        .unwrap();

    // After the step, run should have updated cost/duration from persist_progress
    let run_after = store.get_run(run_id).await.unwrap().unwrap();
    assert!(
        run_after.duration_ms > 0,
        "persist_progress should have updated run duration_ms"
    );
}

#[tokio::test]
async fn step_results_accumulated_in_context() {
    let (mut ctx, _store) = run_ctx().await;

    ctx.shell("step_a", ShellConfig::new("echo a"))
        .await
        .unwrap();
    ctx.shell("step_b", ShellConfig::new("echo b"))
        .await
        .unwrap();

    let results = ctx.step_results();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].name, "step_a");
    assert_eq!(results[0].status, StepStatus::Completed);
    assert_eq!(results[1].name, "step_b");
    assert_eq!(results[1].status, StepStatus::Completed);
    assert!(results[0].output_summary.is_some());
}

#[tokio::test]
async fn step_has_deterministic_trace_id() {
    let (mut ctx, store) = run_ctx().await;
    let run_id = ctx.run_id();

    ctx.shell("build", ShellConfig::new("echo ok"))
        .await
        .unwrap();

    let steps = store.list_steps(run_id).await.unwrap();
    assert_eq!(steps.len(), 1);

    let expected_trace_id = step_trace_id(run_id, "build", 0);
    assert_eq!(steps[0].trace_id, expected_trace_id);
}

#[tokio::test]
async fn failed_step_records_step_result() {
    let (mut ctx, _store) = run_ctx().await;

    let result = ctx.shell("bad_cmd", ShellConfig::new("exit 1")).await;
    assert!(result.is_err());

    let results = ctx.step_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "bad_cmd");
    assert_eq!(results[0].status, StepStatus::Failed);
    assert!(results[0].error.is_some());
}

#[tokio::test]
async fn workflow_result_carries_step_results() {
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

    struct TwoStepWorkflow;

    impl WorkflowHandler for TwoStepWorkflow {
        fn name(&self) -> &str {
            "two-step"
        }
        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.shell("step_a", ShellConfig::new("sleep 0.01")).await?;
                ctx.shell("step_b", ShellConfig::new("sleep 0.01")).await?;
                Ok(())
            })
        }
    }

    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let mut engine = Engine::new(store, provider());
    engine.register(TwoStepWorkflow).unwrap();

    let result = engine
        .run_handler("two-step", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(result.steps.len(), 2);
    assert_eq!(result.steps[0].name, "step_a");
    assert_eq!(result.steps[0].status, StepStatus::Completed);
    assert_eq!(result.steps[1].name, "step_b");
    assert_eq!(result.steps[1].status, StepStatus::Completed);
    assert!(result.run.duration_ms > 0);
}
