//! Integration tests for WorkflowEventBus emission during step execution.

use std::sync::Arc;
use std::time::Duration;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{WorkflowEvent, WorkflowEventBus};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use serde_json::json;
use tokio::time::timeout;

fn create_test_engine() -> (Engine, Arc<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let inner = ClaudeCodeProvider::new();
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ));
    (Engine::new(store.clone(), provider), store)
}

struct EchoWorkflow;

impl WorkflowHandler for EchoWorkflow {
    fn name(&self) -> &str {
        "echo"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("greet", ShellConfig::new("echo hello")).await?;
            Ok(())
        })
    }
}

struct FailingWorkflow;

impl WorkflowHandler for FailingWorkflow {
    fn name(&self) -> &str {
        "failing"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("will-fail", ShellConfig::new("exit 1")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn execute_step_emits_workflow_events() {
    let (mut engine, store) = create_test_engine();
    let bus = WorkflowEventBus::new();
    engine.set_event_bus(bus.clone());
    engine.register(EchoWorkflow).unwrap();

    let run = engine
        .enqueue_handler("echo", TriggerKind::Manual, json!({}), 0)
        .await
        .unwrap();

    // Transition to Running (normally done by the worker).
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();

    let mut rx = bus.subscribe(run.id);

    engine.execute_handler_run(run.id).await.unwrap();

    let mut events = Vec::new();
    while let Ok(Ok(event)) = timeout(Duration::from_millis(100), rx.recv()).await {
        events.push(event);
    }

    assert!(
        events.len() >= 2,
        "expected at least StepStarted + StepCompleted, got {} events",
        events.len()
    );

    assert_eq!(events[0].event_type(), "step_started");
    match &events[0] {
        WorkflowEvent::StepStarted {
            step_name,
            step_index,
            ..
        } => {
            assert_eq!(step_name, "greet");
            assert_eq!(*step_index, 0);
        }
        _ => panic!("expected StepStarted"),
    }

    assert_eq!(events[1].event_type(), "step_completed");
    match &events[1] {
        WorkflowEvent::StepCompleted {
            step_name,
            step_index,
            ..
        } => {
            assert_eq!(step_name, "greet");
            assert_eq!(*step_index, 0);
        }
        _ => panic!("expected StepCompleted"),
    }
}

#[tokio::test]
async fn execute_step_emits_failed_event() {
    let (mut engine, store) = create_test_engine();
    let bus = WorkflowEventBus::new();
    engine.set_event_bus(bus.clone());
    engine.register(FailingWorkflow).unwrap();

    let run = engine
        .enqueue_handler("failing", TriggerKind::Manual, json!({}), 0)
        .await
        .unwrap();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();

    let mut rx = bus.subscribe(run.id);

    let result = engine.execute_handler_run(run.id).await;
    assert!(result.is_err());

    let mut events = Vec::new();
    while let Ok(Ok(event)) = timeout(Duration::from_millis(100), rx.recv()).await {
        events.push(event);
    }

    assert!(
        events.len() >= 2,
        "expected at least StepStarted + StepFailed, got {} events",
        events.len()
    );

    assert_eq!(events[0].event_type(), "step_started");
    assert_eq!(events[1].event_type(), "step_failed");

    match &events[1] {
        WorkflowEvent::StepFailed {
            step_name, error, ..
        } => {
            assert_eq!(step_name, "will-fail");
            assert!(!error.is_empty());
        }
        _ => panic!("expected StepFailed"),
    }
}

#[tokio::test]
async fn engine_passes_event_bus_to_context() {
    let (mut engine, _store) = create_test_engine();
    let bus = WorkflowEventBus::new();
    engine.set_event_bus(bus.clone());

    assert!(engine.event_bus().is_some());
}
