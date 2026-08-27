//! Integration tests for WorkflowEventBus emission during step execution.

use std::sync::Arc;
use std::time::Duration;

use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{AgentStepConfig, ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{WorkflowEvent, WorkflowEventBus};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use rust_decimal::Decimal;
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

struct ApprovalWorkflow;

impl WorkflowHandler for ApprovalWorkflow {
    fn name(&self) -> &str {
        "approval-test"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.approval("deploy-gate", ApprovalConfig::new("Approve deployment?"))
                .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn approval_emits_approval_required_event() {
    let (mut engine, store) = create_test_engine();
    let bus = WorkflowEventBus::new();
    engine.set_event_bus(bus.clone());
    engine.register(ApprovalWorkflow).unwrap();

    let run = engine
        .enqueue_handler("approval-test", TriggerKind::Manual, json!({}), 0)
        .await
        .unwrap();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();

    let mut rx = bus.subscribe(run.id);

    let _result = engine.execute_handler_run(run.id).await.unwrap();

    let mut events = Vec::new();
    while let Ok(Ok(event)) = timeout(Duration::from_millis(100), rx.recv()).await {
        events.push(event);
    }

    let approval_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type() == "approval_required")
        .collect();

    assert_eq!(
        approval_events.len(),
        1,
        "expected exactly one ApprovalRequired event, got {}",
        approval_events.len()
    );

    match &approval_events[0] {
        WorkflowEvent::ApprovalRequired {
            step_name,
            step_index,
            approval_id,
        } => {
            assert_eq!(step_name, "deploy-gate");
            assert_eq!(*step_index, 0);
            assert!(!approval_id.is_nil());
        }
        _ => panic!("expected ApprovalRequired"),
    }
}

/// Stub provider that returns a fixed AgentOutput with token counts.
struct FixedTokenProvider {
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
}

impl AgentProvider for FixedTokenProvider {
    fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
        let input = self.input_tokens;
        let output = self.output_tokens;
        let cost = self.cost_usd;
        Box::pin(async move {
            let mut out = AgentOutput::new(json!({"result": "done"}));
            out.cost_usd = Some(cost);
            out.input_tokens = Some(input);
            out.output_tokens = Some(output);
            out.model = Some("test-model".to_string());
            out.duration_ms = 100;
            Ok(out)
        })
    }
}

struct AgentWorkflow;

impl WorkflowHandler for AgentWorkflow {
    fn name(&self) -> &str {
        "agent-test"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("review", AgentStepConfig::new("Review the code"))
                .await?;
            Ok(())
        })
    }
}

fn create_agent_engine() -> (Engine, Arc<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let provider: Arc<dyn AgentProvider> = Arc::new(FixedTokenProvider {
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.0042,
    });
    (Engine::new(store.clone(), provider), store)
}

#[tokio::test]
async fn agent_step_emits_tokens_used_event() {
    let (mut engine, store) = create_agent_engine();
    let bus = WorkflowEventBus::new();
    engine.set_event_bus(bus.clone());
    engine.register(AgentWorkflow).unwrap();

    let run = engine
        .enqueue_handler("agent-test", TriggerKind::Manual, json!({}), 0)
        .await
        .unwrap();

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

    let token_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type() == "agent_step_tokens_used")
        .collect();

    assert_eq!(
        token_events.len(),
        1,
        "expected exactly one AgentStepTokensUsed event, got {}",
        token_events.len()
    );

    match &token_events[0] {
        WorkflowEvent::AgentStepTokensUsed {
            step_name,
            tokens,
            cost_usd,
        } => {
            assert_eq!(step_name, "review");
            assert_eq!(*tokens, 1500);
            assert_eq!(*cost_usd, Decimal::new(42, 4));
        }
        _ => panic!("expected AgentStepTokensUsed"),
    }

    let event_types: Vec<_> = events.iter().map(|e| e.event_type()).collect();
    assert!(
        event_types.contains(&"step_started"),
        "should also emit StepStarted"
    );
    assert!(
        event_types.contains(&"step_completed"),
        "should also emit StepCompleted"
    );
}
