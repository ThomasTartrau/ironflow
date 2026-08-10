//! Integration tests for on_error handlers.

use std::collections::HashMap;
use std::sync::Arc;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, TriggerKind};
use serde_json::json;

fn create_test_engine() -> Engine {
    let store = Arc::new(InMemoryStore::new());
    let inner = ClaudeCodeProvider::new();
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ));
    Engine::new(store, provider)
}

// ---- Row 1: on_error handler fires on step failure ----

struct FailWithCleanup;

impl WorkflowHandler for FailWithCleanup {
    fn name(&self) -> &str {
        "fail-with-cleanup"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.on_error("cleanup", ShellConfig::new("echo cleaned up"));
            ctx.shell("will-fail", ShellConfig::new("exit 1")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn on_error_fires_on_step_failure() {
    let mut engine = create_test_engine();
    engine.register(FailWithCleanup).unwrap();

    let result = engine
        .run_handler("fail-with-cleanup", TriggerKind::Manual, json!({}))
        .await;

    assert!(result.is_err());

    let runs = engine
        .store()
        .list_runs(Default::default(), 1, 10)
        .await
        .unwrap();
    let run_id = runs.items[0].id;
    let steps = engine.store().list_steps(run_id).await.unwrap();

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].name, "will-fail");
    assert_eq!(steps[0].status.state, StepStatus::Failed);
    assert!(!steps[0].is_error_handler);

    assert_eq!(steps[1].name, "cleanup");
    assert_eq!(steps[1].status.state, StepStatus::Completed);
    assert!(steps[1].is_error_handler);
}

// ---- Row 2: on_error handler does NOT fire on step success ----

struct SuccessWithCleanup;

impl WorkflowHandler for SuccessWithCleanup {
    fn name(&self) -> &str {
        "success-with-cleanup"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.on_error("cleanup", ShellConfig::new("echo cleaned up"));
            ctx.shell("will-pass", ShellConfig::new("echo ok")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn on_error_does_not_fire_on_success() {
    let mut engine = create_test_engine();
    engine.register(SuccessWithCleanup).unwrap();

    let run = engine
        .run_handler("success-with-cleanup", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].name, "will-pass");
    assert_eq!(steps[0].status.state, StepStatus::Completed);
    assert!(!steps[0].is_error_handler);
}

// ---- Row 3: on_error handler failure is swallowed ----

struct FailWithFailingCleanup;

impl WorkflowHandler for FailWithFailingCleanup {
    fn name(&self) -> &str {
        "fail-with-failing-cleanup"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.on_error("bad-cleanup", ShellConfig::new("exit 42"));
            ctx.shell("will-fail", ShellConfig::new("exit 1")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn on_error_handler_failure_swallowed() {
    let mut engine = create_test_engine();
    engine.register(FailWithFailingCleanup).unwrap();

    let result = engine
        .run_handler("fail-with-failing-cleanup", TriggerKind::Manual, json!({}))
        .await;

    assert!(result.is_err());

    let runs = engine
        .store()
        .list_runs(Default::default(), 1, 10)
        .await
        .unwrap();
    let run_id = runs.items[0].id;
    let steps = engine.store().list_steps(run_id).await.unwrap();

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].name, "will-fail");
    assert_eq!(steps[0].status.state, StepStatus::Failed);
    assert!(!steps[0].is_error_handler);

    assert_eq!(steps[1].name, "bad-cleanup");
    assert_eq!(steps[1].status.state, StepStatus::Failed);
    assert!(steps[1].is_error_handler);
}

// ---- Row 4: multiple on_error handlers fire in order ----

struct FailWithMultipleCleanups;

impl WorkflowHandler for FailWithMultipleCleanups {
    fn name(&self) -> &str {
        "fail-with-multiple-cleanups"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.on_error("cleanup-a", ShellConfig::new("echo a"));
            ctx.on_error("cleanup-b", ShellConfig::new("echo b"));
            ctx.shell("will-fail", ShellConfig::new("exit 1")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn on_error_multiple_handlers_fire_in_order() {
    let mut engine = create_test_engine();
    engine.register(FailWithMultipleCleanups).unwrap();

    let result = engine
        .run_handler(
            "fail-with-multiple-cleanups",
            TriggerKind::Manual,
            json!({}),
        )
        .await;

    assert!(result.is_err());

    let runs = engine
        .store()
        .list_runs(Default::default(), 1, 10)
        .await
        .unwrap();
    let run_id = runs.items[0].id;
    let steps = engine.store().list_steps(run_id).await.unwrap();

    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].name, "will-fail");
    assert!(!steps[0].is_error_handler);

    assert_eq!(steps[1].name, "cleanup-a");
    assert!(steps[1].is_error_handler);
    assert_eq!(steps[1].status.state, StepStatus::Completed);

    assert_eq!(steps[2].name, "cleanup-b");
    assert!(steps[2].is_error_handler);
    assert_eq!(steps[2].status.state, StepStatus::Completed);
}

// ---- Row 5: clear_error_handlers removes all handlers ----

struct FailAfterClear;

impl WorkflowHandler for FailAfterClear {
    fn name(&self) -> &str {
        "fail-after-clear"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.on_error("cleanup", ShellConfig::new("echo cleaned"));
            ctx.shell("pass-step", ShellConfig::new("echo ok")).await?;
            ctx.clear_error_handlers();
            ctx.shell("will-fail", ShellConfig::new("exit 1")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn clear_error_handlers_prevents_firing() {
    let mut engine = create_test_engine();
    engine.register(FailAfterClear).unwrap();

    let result = engine
        .run_handler("fail-after-clear", TriggerKind::Manual, json!({}))
        .await;

    assert!(result.is_err());

    let runs = engine
        .store()
        .list_runs(Default::default(), 1, 10)
        .await
        .unwrap();
    let run_id = runs.items[0].id;
    let steps = engine.store().list_steps(run_id).await.unwrap();

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].name, "pass-step");
    assert_eq!(steps[0].status.state, StepStatus::Completed);
    assert_eq!(steps[1].name, "will-fail");
    assert_eq!(steps[1].status.state, StepStatus::Failed);
    // No cleanup step should exist
    assert!(steps.iter().all(|s| !s.is_error_handler));
}

// ---- Row 6: error handler step has is_error_handler=true in store ----
// (covered by on_error_fires_on_step_failure above)

// ---- Row 7: error handler receives error context (env vars for shell) ----

struct FailWithContextCheck;

impl WorkflowHandler for FailWithContextCheck {
    fn name(&self) -> &str {
        "fail-with-context-check"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.on_error(
                "check-context",
                ShellConfig::new("echo $IRONFLOW_ERROR_STEP"),
            );
            ctx.shell("failing-step", ShellConfig::new("exit 1"))
                .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn on_error_receives_error_context() {
    let mut engine = create_test_engine();
    engine.register(FailWithContextCheck).unwrap();

    let result = engine
        .run_handler("fail-with-context-check", TriggerKind::Manual, json!({}))
        .await;

    assert!(result.is_err());

    let runs = engine
        .store()
        .list_runs(Default::default(), 1, 10)
        .await
        .unwrap();
    let run_id = runs.items[0].id;
    let steps = engine.store().list_steps(run_id).await.unwrap();

    let cleanup = steps.iter().find(|s| s.is_error_handler).unwrap();
    assert_eq!(cleanup.name, "check-context");

    let input = cleanup.input.as_ref().unwrap();
    assert_eq!(input["failed_step"], "failing-step");
    assert!(input["error"].as_str().unwrap().len() > 0);
    assert!(input["duration_ms"].is_number());

    let output = cleanup.output.as_ref().unwrap();
    let stdout = output["stdout"].as_str().unwrap();
    assert!(
        stdout.contains("failing-step"),
        "env var IRONFLOW_ERROR_STEP should be set; got stdout: {stdout}"
    );
}
