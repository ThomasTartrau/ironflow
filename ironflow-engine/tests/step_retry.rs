//! Integration tests for step-level retry.
//!
//! Covers the behaviour added by `retry: Option<RetryPolicy>` on step configs:
//! a step with a retry policy retries locally on transient errors, a step that
//! exhausts its retries fails normally, and a step without retry fails
//! immediately.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::retry::RetryPolicy;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, RunStatus, StepStatus, TriggerKind};
use ironflow_store::store::RunStore;

fn engine_with(store: Arc<InMemoryStore>) -> Engine {
    let provider: Arc<dyn AgentProvider> = Arc::new(ClaudeCodeProvider::new());
    Engine::new(store, provider)
}

async fn enqueue(store: &InMemoryStore, workflow: &str) -> Uuid {
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: workflow.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("create run")
        .into_run()
        .id
}

// ---------------------------------------------------------------------------
// Handler: shell step with retry that succeeds on the 2nd attempt
// ---------------------------------------------------------------------------

struct RetrySucceedsOnSecond {
    counter_file: String,
}

impl WorkflowHandler for RetrySucceedsOnSecond {
    fn name(&self) -> &str {
        "retry-succeeds"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // The shell command increments a counter file. On first call (counter
            // file absent or contains "0"), it writes "1" and exits 1. On second
            // call (contains "1"), it exits 0.
            let cmd = format!(
                r#"
                f="{counter_file}"
                if [ ! -f "$f" ]; then echo 0 > "$f"; fi
                c=$(cat "$f")
                if [ "$c" = "0" ]; then
                    echo 1 > "$f"
                    exit 1
                fi
                echo "ok"
                "#,
                counter_file = self.counter_file
            );

            let config = ShellConfig::new(&cmd)
                .retry_policy(RetryPolicy::new(2).backoff(std::time::Duration::from_millis(10)));
            ctx.shell("flaky-step", config).await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Handler: shell step with retry that exhausts all retries
// ---------------------------------------------------------------------------

struct RetryExhausted;

impl WorkflowHandler for RetryExhausted {
    fn name(&self) -> &str {
        "retry-exhausted"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let config = ShellConfig::new("exit 1")
                .retry_policy(RetryPolicy::new(2).backoff(std::time::Duration::from_millis(10)));
            ctx.shell("always-fails", config).await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Handler: shell step without retry
// ---------------------------------------------------------------------------

struct NoRetry;

impl WorkflowHandler for NoRetry {
    fn name(&self) -> &str {
        "no-retry"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("instant-fail", ShellConfig::new("exit 1"))
                .await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn step_with_retry_succeeds_on_second_attempt() {
    let counter_file = format!(
        "/tmp/ironflow_test_step_retry_{}",
        Uuid::now_v7().as_simple()
    );
    // Ensure the file does not exist from a previous run.
    let _ = std::fs::remove_file(&counter_file);

    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());

    engine
        .register(RetrySucceedsOnSecond {
            counter_file: counter_file.clone(),
        })
        .expect("register");

    let run_id = enqueue(&store, "retry-succeeds").await;
    store.pick_next_pending(None).await.unwrap().unwrap();
    engine.execute_handler_run(run_id).await.expect("execute");

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = store.list_steps(run_id).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status.state, StepStatus::Completed);

    // Cleanup
    let _ = std::fs::remove_file(&counter_file);
}

#[tokio::test]
async fn step_exhausts_retries_then_fails() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());

    engine.register(RetryExhausted).expect("register");

    let run_id = enqueue(&store, "retry-exhausted").await;
    store.pick_next_pending(None).await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Failed);

    let steps = store.list_steps(run_id).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status.state, StepStatus::Failed);
}

#[tokio::test]
async fn step_without_retry_fails_immediately() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());

    engine.register(NoRetry).expect("register");

    let run_id = enqueue(&store, "no-retry").await;
    store.pick_next_pending(None).await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Failed);

    let steps = store.list_steps(run_id).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status.state, StepStatus::Failed);
}
