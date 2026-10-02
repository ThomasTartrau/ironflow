//! Integration tests for signal steps (`ctx.wait_for_signal::<S>()`).
//!
//! Every test drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`]. Signals are delivered through
//! [`Engine::deliver_signal`] / [`Engine::send_signal`], the way the API server
//! does, and woken runs resume in-process ([`ExecutionMode::Local`]).
//!
//! Test names contain `signal` so `cargo test -p ironflow-engine signal`
//! selects the whole suite.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{Event, EventSubscriber, SubscriberFuture};
use ironflow_engine::plan::PlanOptions;
use ironflow_engine::signal::Signal;
use ironflow_engine::testing::{SignalOutcome, TestEngine};
use ironflow_engine::wake::RunWaker;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    NewSignal, RunStatus, RunUpdate, SignalFilter, Step, StepKind, StepStatus, TriggerKind,
};
use ironflow_store::signal_store::SignalStore;
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that wait for a resumed run.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`WaitCi`].
const WORKFLOW: &str = "wait-ci";

/// The signal the handler waits for.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PipelineFinished {
    status: String,
}

impl Signal for PipelineFinished {
    const NAME: &'static str = "ci.pipeline_finished";
}

/// The run payload: the commit the handler waits on, and for how long.
#[derive(Deserialize)]
struct Input {
    sha: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    3_600_000
}

/// What the handler received: the pipeline status, or `None` on timeout.
type Seen = Arc<Mutex<Vec<Option<String>>>>;

/// Waits for [`PipelineFinished`] on the payload's commit and records it.
struct WaitCi {
    seen: Seen,
}

impl WorkflowHandler for WaitCi {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: Input = ctx.input().await?;
            let finished = ctx
                .wait_for_signal::<PipelineFinished>(
                    "wait-ci",
                    &input.sha,
                    Duration::from_millis(input.timeout_ms),
                )
                .await?;
            self.seen
                .lock()
                .expect("seen lock")
                .push(finished.map(|f| f.status));
            Ok(())
        })
    }
}

/// Waits with an empty key: the step panics before touching the store.
struct EmptyKey;

impl WorkflowHandler for EmptyKey {
    fn name(&self) -> &str {
        "empty-key"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.wait_for_signal::<PipelineFinished>("wait", "  ", Duration::from_secs(60))
                .await?;
            Ok(())
        })
    }
}

/// Waits with a zero timeout: the step panics before touching the store.
struct ZeroTimeout;

impl WorkflowHandler for ZeroTimeout {
    fn name(&self) -> &str {
        "zero-timeout"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.wait_for_signal::<PipelineFinished>("wait", "abc", Duration::ZERO)
                .await?;
            Ok(())
        })
    }
}

/// Collects every published event.
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

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        "/tmp/ironflow-fixtures",
    ))
}

fn new_engine(store: Arc<InMemoryStore>, seen: Seen) -> Engine {
    let store: Arc<dyn Store> = store;
    let mut engine = Engine::new(store, provider());
    engine.register(WaitCi { seen }).expect("register handler");
    engine
}

fn setup() -> (Arc<Engine>, Arc<InMemoryStore>, Seen) {
    let store = Arc::new(InMemoryStore::new());
    let seen = Seen::default();
    let engine = Arc::new(new_engine(store.clone(), seen.clone()));
    (engine, store, seen)
}

fn pipeline_signal(sha: &str, status: &str, idempotency_id: Option<&str>) -> NewSignal {
    NewSignal {
        name: PipelineFinished::NAME.to_string(),
        key: sha.to_string(),
        payload: json!({ "status": status }),
        idempotency_id: idempotency_id.map(str::to_string),
    }
}

/// Run the handler until it suspends, returning the run id.
async fn start_waiting(engine: &Engine, sha: &str) -> Uuid {
    let result = engine
        .run_handler(WORKFLOW, TriggerKind::Manual, json!({ "sha": sha }))
        .await
        .expect("handler suspends on the signal");
    assert_eq!(result.run.status.state, RunStatus::Sleeping);
    result.run.id
}

/// Poll the store until the run reaches `status`.
async fn wait_for_status(store: &InMemoryStore, run_id: Uuid, status: RunStatus) {
    timeout(TEST_TIMEOUT, async {
        loop {
            let run = store
                .get_run(run_id)
                .await
                .expect("get run")
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

/// The single signal step of a run.
async fn signal_step(store: &InMemoryStore, run_id: Uuid) -> Step {
    let steps = store.list_steps(run_id).await.expect("list steps");
    let mut signal_steps: Vec<_> = steps
        .into_iter()
        .filter(|s| s.kind == StepKind::Signal)
        .collect();
    assert_eq!(signal_steps.len(), 1, "exactly one signal step");
    signal_steps.remove(0)
}

#[tokio::test]
async fn signal_delivered_resumes_waiting_run() {
    let (engine, store, seen) = setup();
    let run_id = start_waiting(&engine, "abc123").await;

    let step = signal_step(&store, run_id).await;
    assert_eq!(step.status.state, StepStatus::Running);
    let run = store.get_run(run_id).await.unwrap().expect("run exists");
    assert!(run.scheduled_at.is_some(), "the deadline is armed");

    let delivery = engine
        .send_signal(
            &PipelineFinished {
                status: "success".to_string(),
            },
            "abc123",
            None,
        )
        .await
        .expect("deliver");
    assert!(!delivery.duplicate);
    assert!(delivery.rejected.is_empty());
    assert_eq!(delivery.resumed.len(), 1);
    assert_eq!(delivery.resumed[0].run_id, run_id);
    assert_eq!(delivery.resumed[0].step_id, step.id);

    wait_for_status(&store, run_id, RunStatus::Completed).await;
    assert_eq!(*seen.lock().unwrap(), vec![Some("success".to_string())]);

    let step = signal_step(&store, run_id).await;
    assert_eq!(step.status.state, StepStatus::Completed);
    let output = step.output.expect("resolved step has an output");
    assert_eq!(output["timed_out"], json!(false));
    assert_eq!(output["payload"], json!({"status": "success"}));
    assert_eq!(output["signal_id"], json!(delivery.signal_id));
}

#[tokio::test]
async fn signal_received_before_wait_completes_without_sleeping() {
    let (engine, store, seen) = setup();
    let run = engine
        .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({"sha": "early"}), 0)
        .await
        .expect("enqueue");

    let delivery = engine
        .deliver_signal(pipeline_signal("early", "success", None))
        .await
        .expect("deliver");
    assert!(delivery.resumed.is_empty(), "nobody waits yet");

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("mark running");
    let result = engine
        .execute_handler_run(run.id)
        .await
        .expect("handler completes");

    assert_eq!(result.run.status.state, RunStatus::Completed);
    assert_eq!(*seen.lock().unwrap(), vec![Some("success".to_string())]);
    let step = signal_step(&store, run.id).await;
    assert_eq!(step.status.state, StepStatus::Completed);
}

#[tokio::test]
async fn signal_sent_before_the_run_existed_is_ignored() {
    let (engine, _store, _seen) = setup();
    engine
        .deliver_signal(pipeline_signal("stale", "success", None))
        .await
        .expect("deliver");

    // The run is created after the signal: it must wait for a new one.
    start_waiting(&engine, "stale").await;
}

#[tokio::test]
async fn signal_timeout_returns_none() {
    let (engine, store, seen) = setup();
    let result = engine
        .run_handler(
            WORKFLOW,
            TriggerKind::Manual,
            json!({"sha": "slow", "timeout_ms": 50}),
        )
        .await
        .expect("handler suspends on the signal");
    assert_eq!(result.run.status.state, RunStatus::Sleeping);
    let run_id = result.run.id;

    // Let the deadline pass: the waker only claims a run once it is due.
    sleep(Duration::from_millis(100)).await;
    let woken = RunWaker::new(engine.clone()).tick().await.expect("tick");
    assert_eq!(woken.iter().map(|r| r.id).collect::<Vec<_>>(), vec![run_id]);

    wait_for_status(&store, run_id, RunStatus::Completed).await;
    assert_eq!(*seen.lock().unwrap(), vec![None]);
    let step = signal_step(&store, run_id).await;
    assert_eq!(step.status.state, StepStatus::Completed);
    assert_eq!(step.output, Some(json!({"timed_out": true})));
}

#[tokio::test]
async fn signal_woken_before_deadline_suspends_again() {
    let (engine, store, seen) = setup();
    let run_id = start_waiting(&engine, "early-wake").await;
    let input = signal_step(&store, run_id).await.input.expect("input");
    let deadline = input["deadline_at"].clone();

    store
        .update_run(
            run_id,
            RunUpdate {
                scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                ..RunUpdate::default()
            },
        )
        .await
        .expect("wake early");
    RunWaker::new(engine.clone()).tick().await.expect("tick");

    wait_for_status(&store, run_id, RunStatus::Sleeping).await;
    assert!(seen.lock().unwrap().is_empty());
    let run = store.get_run(run_id).await.unwrap().expect("run exists");
    assert_eq!(json!(run.scheduled_at.expect("re-armed")), deadline);
}

#[tokio::test]
async fn signal_with_invalid_payload_is_rejected() {
    let (engine, store, seen) = setup();
    let run_id = start_waiting(&engine, "bad").await;

    let delivery = engine
        .deliver_signal(NewSignal {
            name: PipelineFinished::NAME.to_string(),
            key: "bad".to_string(),
            payload: json!({"status": 42}),
            idempotency_id: None,
        })
        .await
        .expect("deliver");
    assert!(delivery.resumed.is_empty());
    assert_eq!(delivery.rejected.len(), 1);
    assert_eq!(delivery.rejected[0].run_id, run_id);

    let run = store.get_run(run_id).await.unwrap().expect("run exists");
    assert_eq!(run.status.state, RunStatus::Sleeping);
    assert_eq!(
        signal_step(&store, run_id).await.status.state,
        StepStatus::Running
    );
    assert!(seen.lock().unwrap().is_empty());

    let stored = store
        .list_signals(
            SignalFilter {
                name: Some(PipelineFinished::NAME.to_string()),
                key: Some("bad".to_string()),
            },
            1,
            20,
        )
        .await
        .expect("list signals");
    assert_eq!(stored.total, 1, "an invalid signal is still stored");
}

#[tokio::test]
async fn signal_duplicate_idempotency_id_is_not_redelivered() {
    let (engine, store, _seen) = setup();
    let first_run = start_waiting(&engine, "dup").await;

    let first = engine
        .deliver_signal(pipeline_signal("dup", "success", Some("delivery-1")))
        .await
        .expect("deliver");
    assert!(!first.duplicate);
    assert_eq!(first.resumed.len(), 1);
    wait_for_status(&store, first_run, RunStatus::Completed).await;

    let second_run = start_waiting(&engine, "dup-other").await;
    let second = engine
        .deliver_signal(pipeline_signal("dup-other", "success", Some("delivery-1")))
        .await
        .expect("deliver again");
    assert!(second.duplicate);
    assert_eq!(second.signal_id, first.signal_id);
    assert!(second.resumed.is_empty());

    let run = store
        .get_run(second_run)
        .await
        .unwrap()
        .expect("run exists");
    assert_eq!(run.status.state, RunStatus::Sleeping);
}

#[tokio::test]
async fn signal_broadcast_resumes_every_waiting_run() {
    let (engine, store, seen) = setup();
    let first = start_waiting(&engine, "shared").await;
    let second = start_waiting(&engine, "shared").await;
    let other = start_waiting(&engine, "other").await;

    let delivery = engine
        .deliver_signal(pipeline_signal("shared", "success", None))
        .await
        .expect("deliver");
    let mut resumed: Vec<Uuid> = delivery.resumed.iter().map(|r| r.run_id).collect();
    resumed.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(resumed, expected);

    wait_for_status(&store, first, RunStatus::Completed).await;
    wait_for_status(&store, second, RunStatus::Completed).await;
    assert_eq!(seen.lock().unwrap().len(), 2);
    let run = store.get_run(other).await.unwrap().expect("run exists");
    assert_eq!(run.status.state, RunStatus::Sleeping);
}

#[tokio::test]
async fn signal_cancelled_run_is_not_resumed() {
    let (engine, store, _seen) = setup();
    let run_id = start_waiting(&engine, "cancelled").await;
    store
        .update_run_status(run_id, RunStatus::Cancelled)
        .await
        .expect("cancel");

    let delivery = engine
        .deliver_signal(pipeline_signal("cancelled", "success", None))
        .await
        .expect("deliver");
    assert!(delivery.resumed.is_empty());
    assert!(delivery.rejected.is_empty());
}

#[tokio::test]
async fn signal_with_empty_key_is_invalid() {
    let (engine, store, _seen) = setup();
    let err = engine
        .deliver_signal(pipeline_signal(" ", "success", None))
        .await
        .expect_err("an empty key is refused");
    assert!(matches!(err, EngineError::InvalidSignal(_)), "got {err:?}");

    let err = engine
        .deliver_signal(NewSignal {
            name: String::new(),
            ..pipeline_signal("abc", "success", None)
        })
        .await
        .expect_err("an empty name is refused");
    assert!(matches!(err, EngineError::InvalidSignal(_)), "got {err:?}");

    let stored = store
        .list_signals(SignalFilter::default(), 1, 20)
        .await
        .expect("list signals");
    assert_eq!(stored.total, 0, "nothing is stored");
}

#[tokio::test]
async fn signal_in_plan_mode_does_not_suspend() {
    let (engine, store, seen) = setup();
    let plan = engine
        .plan_handler(WORKFLOW, json!({"sha": "abc"}), PlanOptions::default())
        .await
        .expect("plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].name, "wait-ci");
    assert_eq!(plan.steps[0].kind, StepKind::Signal);
    // `PipelineFinished` cannot be built from `{}`: the handler sees `None`.
    assert_eq!(*seen.lock().unwrap(), vec![None]);
    let signals = store
        .list_signals(SignalFilter::default(), 1, 20)
        .await
        .expect("list signals");
    assert_eq!(signals.total, 0);
}

#[tokio::test]
async fn signal_mock_interceptor_provides_value() {
    let seen = Seen::default();
    let result = TestEngine::new()
        .with_handler(WaitCi { seen: seen.clone() })
        .with_mock_signal(|step, name, key| {
            assert_eq!(step, "wait-ci");
            assert_eq!(name, PipelineFinished::NAME);
            assert_eq!(key, "abc");
            SignalOutcome::Received(json!({"status": "success"}))
        })
        .run(json!({"sha": "abc"}))
        .await
        .expect("run");

    assert!(result.is_completed());
    assert_eq!(*seen.lock().unwrap(), vec![Some("success".to_string())]);
    let step = result.step("wait-ci");
    assert_eq!(step.kind(), &StepKind::Signal);
    assert_eq!(step.output()["payload"], json!({"status": "success"}));
}

#[tokio::test]
async fn signal_mock_interceptor_forces_timeout() {
    let seen = Seen::default();
    let result = TestEngine::new()
        .with_handler(WaitCi { seen: seen.clone() })
        .with_mock_signal(|_step, _name, _key| SignalOutcome::TimedOut)
        .run(json!({"sha": "abc"}))
        .await
        .expect("run");

    assert!(result.is_completed());
    assert_eq!(*seen.lock().unwrap(), vec![None]);
    assert_eq!(result.step("wait-ci").output(), &json!({"timed_out": true}));
}

#[tokio::test]
#[should_panic(expected = "wait_for_signal: key must not be empty")]
async fn wait_for_signal_panics_on_empty_key() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let mut engine = Engine::new(store, provider());
    engine.register(EmptyKey).expect("register handler");
    let _ = engine
        .run_handler("empty-key", TriggerKind::Manual, json!({}))
        .await;
}

#[tokio::test]
#[should_panic(expected = "wait_for_signal: timeout must be greater than zero")]
async fn wait_for_signal_panics_on_zero_timeout() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let mut engine = Engine::new(store, provider());
    engine.register(ZeroTimeout).expect("register handler");
    let _ = engine
        .run_handler("zero-timeout", TriggerKind::Manual, json!({}))
        .await;
}

#[tokio::test]
async fn signal_received_publishes_events() {
    let store = Arc::new(InMemoryStore::new());
    let seen = Seen::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut engine = new_engine(store.clone(), seen);
    engine.subscribe(
        EventCollector {
            events: events.clone(),
        },
        &[Event::SIGNAL_AWAITED, Event::SIGNAL_RECEIVED],
    );
    let engine = Arc::new(engine);

    let run_id = start_waiting(&engine, "evt").await;
    let delivery = engine
        .deliver_signal(pipeline_signal("evt", "success", None))
        .await
        .expect("deliver");
    wait_for_status(&store, run_id, RunStatus::Completed).await;

    timeout(TEST_TIMEOUT, async {
        loop {
            if events.lock().unwrap().len() >= 2 {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both events are published");

    let events = events.lock().unwrap();
    let awaited = events
        .iter()
        .find_map(|e| match e {
            Event::SignalAwaited(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("a signal_awaited event");
    assert_eq!(awaited.run_id, run_id);
    assert_eq!(awaited.step_name, "wait-ci");
    assert_eq!(awaited.name, PipelineFinished::NAME);
    assert_eq!(awaited.key, "evt");

    let received = events
        .iter()
        .find_map(|e| match e {
            Event::SignalReceived(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("a signal_received event");
    assert_eq!(received.signal_id, delivery.signal_id);
    assert_eq!(received.resumed_runs, vec![run_id]);
}
