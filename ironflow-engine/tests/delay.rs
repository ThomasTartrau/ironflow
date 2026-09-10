//! Integration tests for delay (timed pause) steps.

use std::sync::Arc;

use serde_json::json;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::config::delay::DelayConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, TriggerKind};

fn create_test_engine() -> Engine {
    let store = Arc::new(InMemoryStore::new());
    let inner = ClaudeCodeProvider::new();
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ));
    Engine::new(store, provider)
}

struct DelayZeroWorkflow;

impl WorkflowHandler for DelayZeroWorkflow {
    fn name(&self) -> &str {
        "delay-zero"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("before", ShellConfig::new("echo before")).await?;
            ctx.delay("instant", DelayConfig::from_secs(0)).await?;
            ctx.shell("after", ShellConfig::new("echo after")).await?;
            Ok(())
        })
    }
}

struct DelaySleepWorkflow;

impl WorkflowHandler for DelaySleepWorkflow {
    fn name(&self) -> &str {
        "delay-sleep"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("before", ShellConfig::new("echo before")).await?;
            ctx.delay("wait-5min", DelayConfig::from_secs(300)).await?;
            ctx.shell("after", ShellConfig::new("echo after")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn delay_zero_completes_immediately() {
    let mut engine = create_test_engine();
    engine.register(DelayZeroWorkflow).unwrap();

    let result = engine
        .run_handler("delay-zero", TriggerKind::Api, json!({}))
        .await
        .unwrap();

    assert_eq!(result.run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(result.run.id).await.unwrap();
    let step_names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
    assert!(step_names.contains(&"before"));
    assert!(step_names.contains(&"instant"));
    assert!(step_names.contains(&"after"));
}

#[tokio::test]
async fn delay_sleep_transitions_to_sleeping() {
    let mut engine = create_test_engine();
    engine.register(DelaySleepWorkflow).unwrap();

    let result = engine
        .run_handler("delay-sleep", TriggerKind::Api, json!({}))
        .await
        .unwrap();

    assert_eq!(result.run.status.state, RunStatus::Sleeping);

    let steps = engine.store().list_steps(result.run.id).await.unwrap();
    let step_names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
    assert!(step_names.contains(&"before"));
    assert!(step_names.contains(&"wait-5min"));
    assert!(!step_names.contains(&"after"));

    assert!(result.run.scheduled_at.is_some());
}
