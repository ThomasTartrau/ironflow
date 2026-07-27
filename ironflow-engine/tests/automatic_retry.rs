//! Integration tests for automatic run retry.
//!
//! Covers the behaviour promised by `max_retries`: a run that fails on a
//! transient error is replayed after a backoff, a run that fails on something
//! replay cannot fix is not, and every attempt keeps its own steps.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use serde_json::json;
use tokio::time::sleep;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{Event, EventSubscriber, SubscriberFuture};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, RunStatus, RunUpdate, StepKind, StepStatus, TriggerKind};
use ironflow_store::store::RunStore;

/// Records every event it receives, so tests can assert on what was published.
#[derive(Clone, Default)]
struct EventCollector {
    events: Arc<Mutex<Vec<Event>>>,
}

impl EventSubscriber for EventCollector {
    fn name(&self) -> &str {
        "event-collector"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        let events = self.events.clone();
        let event = event.clone();
        Box::pin(async move {
            events.lock().expect("collector lock").push(event);
        })
    }
}

fn engine_with(store: Arc<InMemoryStore>) -> Engine {
    let provider: Arc<dyn AgentProvider> = Arc::new(ClaudeCodeProvider::new());
    Engine::new(store, provider)
}

async fn enqueue(store: &InMemoryStore, workflow: &str, max_retries: u32) -> uuid::Uuid {
    store
        .create_run(NewRun {
            workflow_name: workflow.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
        })
        .await
        .expect("create run")
        .id
}

/// Move a run armed for retry to the front of the queue, as if its backoff had
/// elapsed, then pick it up like a worker would.
async fn fast_forward_backoff(store: &InMemoryStore, run_id: uuid::Uuid) {
    store
        .update_run(
            run_id,
            RunUpdate {
                scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                ..RunUpdate::default()
            },
        )
        .await
        .expect("rewind scheduled_at");

    let picked = store
        .pick_next_pending()
        .await
        .expect("pick")
        .expect("a run waiting for its retry");
    assert_eq!(picked.id, run_id);
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Fails on every attempt with a transient HTTP error.
struct AlwaysTransientlyFails;

impl WorkflowHandler for AlwaysTransientlyFails {
    fn name(&self) -> &str {
        "always-fails"
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            Err(EngineError::Operation(
                ironflow_core::error::OperationError::Http {
                    status: Some(503),
                    message: "upstream unavailable".to_string(),
                },
            ))
        })
    }
}

/// Fails with an error that replaying cannot fix.
struct FailsPermanently;

impl WorkflowHandler for FailsPermanently {
    fn name(&self) -> &str {
        "fails-permanently"
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            Err(EngineError::StepConfig(
                "payload is missing required field 'env'".to_string(),
            ))
        })
    }
}

/// Records a step, then fails on the first attempt only.
struct FailsOnceThenSucceeds {
    attempts: AtomicU32,
}

impl WorkflowHandler for FailsOnceThenSucceeds {
    fn name(&self) -> &str {
        "flaky"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("greet", ShellConfig::new("echo hello")).await?;

            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(EngineError::Operation(
                    ironflow_core::error::OperationError::Http {
                        status: Some(500),
                        message: "flaky upstream".to_string(),
                    },
                ));
            }
            Ok(())
        })
    }
}

/// Gated on a human approval, then fails on the first attempt only.
struct ApprovalThenFailsOnce {
    attempts: AtomicU32,
}

impl WorkflowHandler for ApprovalThenFailsOnce {
    fn name(&self) -> &str {
        "gated"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.approval("deploy-gate", ApprovalConfig::new("Approve deployment?"))
                .await?;

            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(EngineError::Operation(
                    ironflow_core::error::OperationError::Http {
                        status: Some(502),
                        message: "bad gateway".to_string(),
                    },
                ));
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transient_failure_is_replayed_until_max_retries() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine.register(AlwaysTransientlyFails).expect("register");

    let run_id = enqueue(&store, "always-fails", 2).await;

    // Attempt 1.
    store.pick_next_pending().await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());
    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Retrying);
    assert_eq!(run.retry_count, 1);
    assert!(run.scheduled_at.is_some(), "backoff must be armed");

    // Attempt 2.
    fast_forward_backoff(&store, run_id).await;
    assert!(engine.execute_handler_run(run_id).await.is_err());
    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Retrying);
    assert_eq!(run.retry_count, 2);

    // Attempt 3: no attempt left.
    fast_forward_backoff(&store, run_id).await;
    assert!(engine.execute_handler_run(run_id).await.is_err());
    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Failed);
    assert_eq!(run.retry_count, 2);
    assert!(run.completed_at.is_some());
}

#[tokio::test]
async fn backoff_is_in_the_future() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine.register(AlwaysTransientlyFails).expect("register");

    let run_id = enqueue(&store, "always-fails", 1).await;
    store.pick_next_pending().await.unwrap().unwrap();
    let before = Utc::now();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    let run = store.get_run(run_id).await.unwrap().unwrap();
    let scheduled_at = run.scheduled_at.expect("backoff armed");
    assert!(
        scheduled_at > before + TimeDelta::seconds(20),
        "first retry must wait ~30s, got {scheduled_at}"
    );
    assert!(scheduled_at < before + TimeDelta::seconds(40));

    // The run must not be picked up before its backoff elapses.
    assert!(store.pick_next_pending().await.unwrap().is_none());
}

#[tokio::test]
async fn non_retryable_failure_consumes_no_attempt() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine.register(FailsPermanently).expect("register");

    let run_id = enqueue(&store, "fails-permanently", 3).await;
    store.pick_next_pending().await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Failed);
    assert_eq!(run.retry_count, 0);
    assert!(run.scheduled_at.is_none());
}

#[tokio::test]
async fn max_retries_zero_fails_immediately() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine.register(AlwaysTransientlyFails).expect("register");

    let run_id = enqueue(&store, "always-fails", 0).await;
    store.pick_next_pending().await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Failed);
    assert_eq!(run.retry_count, 0);
}

#[tokio::test]
async fn each_attempt_keeps_its_own_steps() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine
        .register(FailsOnceThenSucceeds {
            attempts: AtomicU32::new(0),
        })
        .expect("register");

    let run_id = enqueue(&store, "flaky", 1).await;

    store.pick_next_pending().await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    fast_forward_backoff(&store, run_id).await;
    engine
        .execute_handler_run(run_id)
        .await
        .expect("second attempt succeeds");

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Completed);
    assert_eq!(run.retry_count, 1);

    let steps = store.list_steps(run_id).await.unwrap();
    let greets: Vec<_> = steps.iter().filter(|s| s.name == "greet").collect();
    assert_eq!(greets.len(), 2, "both attempts must be inspectable");

    let mut attempts: Vec<u32> = greets.iter().map(|s| s.attempt).collect();
    attempts.sort_unstable();
    assert_eq!(attempts, vec![1, 2]);
}

#[tokio::test]
async fn retry_does_not_replay_previous_attempt_steps() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine
        .register(FailsOnceThenSucceeds {
            attempts: AtomicU32::new(0),
        })
        .expect("register");

    let run_id = enqueue(&store, "flaky", 1).await;
    store.pick_next_pending().await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    fast_forward_backoff(&store, run_id).await;
    engine.execute_handler_run(run_id).await.expect("retry");

    // The second attempt really executed its step rather than replaying the
    // first attempt's record: it has its own completed step.
    let steps = store.list_steps(run_id).await.unwrap();
    let second = steps
        .iter()
        .find(|s| s.name == "greet" && s.attempt == 2)
        .expect("attempt 2 recorded its own step");
    assert_eq!(second.status.state, StepStatus::Completed);
}

#[tokio::test]
async fn approval_granted_in_a_previous_attempt_is_not_asked_again() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine
        .register(ApprovalThenFailsOnce {
            attempts: AtomicU32::new(0),
        })
        .expect("register");

    let run_id = enqueue(&store, "gated", 1).await;

    // Attempt 1 suspends on the approval gate.
    store.pick_next_pending().await.unwrap().unwrap();
    engine.execute_handler_run(run_id).await.expect("suspends");
    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::AwaitingApproval);

    // A human approves, and the resumed attempt fails past the gate.
    store
        .update_run_status(run_id, RunStatus::Running)
        .await
        .unwrap();
    assert!(engine.resume_run(run_id).await.is_err());

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Retrying);

    // Attempt 2 must run to completion without suspending again.
    fast_forward_backoff(&store, run_id).await;
    engine
        .execute_handler_run(run_id)
        .await
        .expect("retry runs past the approval gate");

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = store.list_steps(run_id).await.unwrap();
    let carried = steps
        .iter()
        .find(|s| s.kind == StepKind::Approval && s.attempt == 2)
        .expect("attempt 2 records the carried-over approval");
    assert_eq!(carried.status.state, StepStatus::Completed);
    assert_eq!(carried.output, Some(json!({"approved_in_attempt": 1})));
}

#[tokio::test]
async fn scheduling_a_retry_does_not_publish_run_failed() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine.register(AlwaysTransientlyFails).expect("register");

    let collector = EventCollector::default();
    engine.subscribe(collector.clone(), Event::ALL);

    let run_id = enqueue(&store, "always-fails", 1).await;
    store.pick_next_pending().await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    // Subscribers run in spawned tasks; let them drain.
    sleep(Duration::from_millis(50)).await;

    let events = collector.events.lock().expect("collector lock").clone();
    assert!(
        !events.iter().any(|e| matches!(e, Event::RunFailed { .. })),
        "a scheduled retry is not a run failure"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::RunStatusChanged {
                to: RunStatus::Retrying,
                ..
            }
        )),
        "the move to Retrying must be published"
    );
}

#[tokio::test]
async fn duration_accumulates_across_attempts() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine.register(AlwaysTransientlyFails).expect("register");

    let run_id = enqueue(&store, "always-fails", 1).await;
    store.pick_next_pending().await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    // Pretend the first attempt took a while.
    store
        .update_run(
            run_id,
            RunUpdate {
                duration_ms: Some(5_000),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    fast_forward_backoff(&store, run_id).await;
    assert!(engine.execute_handler_run(run_id).await.is_err());

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert!(
        run.duration_ms >= 5_000,
        "the second attempt must add to the first attempt's duration, got {}",
        run.duration_ms
    );
}

#[tokio::test]
async fn cost_accumulates_across_attempts() {
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone());
    engine.register(AlwaysTransientlyFails).expect("register");

    let run_id = enqueue(&store, "always-fails", 1).await;
    store.pick_next_pending().await.unwrap().unwrap();
    assert!(engine.execute_handler_run(run_id).await.is_err());

    // Pretend the first attempt spent something.
    store
        .update_run(
            run_id,
            RunUpdate {
                cost_usd: Some(rust_decimal::Decimal::new(150, 2)),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    fast_forward_backoff(&store, run_id).await;
    assert!(engine.execute_handler_run(run_id).await.is_err());

    let run = store.get_run(run_id).await.unwrap().unwrap();
    assert_eq!(
        run.cost_usd,
        rust_decimal::Decimal::new(150, 2),
        "the second attempt must not wipe the first attempt's cost"
    );
}
