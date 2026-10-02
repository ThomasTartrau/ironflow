//! Integration tests for delay (timed pause) steps.

use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use serde_json::json;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::config::delay::DelayConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::wake::RunWaker;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, RunUpdate, TriggerKind};

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

/// Poll the store until the run reaches `status`, failing after 10 seconds.
async fn wait_for_status(engine: &Engine, run_id: Uuid, status: RunStatus) {
    timeout(Duration::from_secs(10), async {
        loop {
            let run = engine
                .store()
                .get_run(run_id)
                .await
                .unwrap()
                .expect("run exists");
            if run.status.state == status {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run never reached the expected status");
}

/// Run `DelaySleepWorkflow` and move its wake-up time to the past.
async fn sleeping_run_due_now(engine: &Engine) -> Uuid {
    let result = engine
        .run_handler("delay-sleep", TriggerKind::Api, json!({}))
        .await
        .unwrap();
    assert_eq!(result.run.status.state, RunStatus::Sleeping);
    engine
        .store()
        .update_run(
            result.run.id,
            RunUpdate {
                scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();
    result.run.id
}

#[tokio::test]
async fn delay_sleeping_run_resumes_after_scheduled_at() {
    let mut engine = create_test_engine();
    engine.register(DelaySleepWorkflow).unwrap();
    let engine = Arc::new(engine);
    let run_id = sleeping_run_due_now(&engine).await;

    let woken = RunWaker::new(engine.clone()).tick().await.unwrap();
    assert_eq!(woken.iter().map(|r| r.id).collect::<Vec<_>>(), vec![run_id]);

    wait_for_status(&engine, run_id, RunStatus::Completed).await;

    let steps = engine.store().list_steps(run_id).await.unwrap();
    let step_names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(step_names, vec!["before", "wait-5min", "after"]);
}

#[tokio::test]
async fn delay_sleeping_run_is_not_woken_before_scheduled_at() {
    let mut engine = create_test_engine();
    engine.register(DelaySleepWorkflow).unwrap();
    let engine = Arc::new(engine);

    let result = engine
        .run_handler("delay-sleep", TriggerKind::Api, json!({}))
        .await
        .unwrap();

    let woken = RunWaker::new(engine.clone()).tick().await.unwrap();
    assert!(woken.is_empty());

    let run = engine
        .store()
        .get_run(result.run.id)
        .await
        .unwrap()
        .expect("run exists");
    assert_eq!(run.status.state, RunStatus::Sleeping);
    assert!(run.scheduled_at.is_some());
}

#[tokio::test]
async fn delay_wake_requeues_to_pending_in_workers_mode() {
    let mut engine = create_test_engine().with_execution_mode(ExecutionMode::Workers);
    engine.register(DelaySleepWorkflow).unwrap();
    let engine = Arc::new(engine);
    let run_id = sleeping_run_due_now(&engine).await;

    let woken = RunWaker::new(engine.clone()).tick().await.unwrap();
    assert_eq!(woken.len(), 1);

    let run = engine
        .store()
        .get_run(run_id)
        .await
        .unwrap()
        .expect("run exists");
    assert_eq!(run.status.state, RunStatus::Pending);
    assert!(run.scheduled_at.is_none());

    let picked = engine
        .store()
        .pick_next_pending(None)
        .await
        .unwrap()
        .expect("the woken run is pending");
    assert_eq!(picked.id, run_id);
}
