//! An agent step interrupted by a lost lease resumes its Claude Code session
//! instead of starting the agent from scratch.
//!
//! A worker crash is simulated for real, like in `lease_lost.rs`: the run is
//! picked with a lease that expires at once, executed on a spawned task, and
//! the task is aborted while the agent hangs. The lease is reaped and the
//! run's open steps are interrupted before another execution picks the run
//! up. A real provider records every config it receives, so the tests assert
//! on the session the engine pinned and on the one it resumed. Test names
//! start with `agent_resume_` so `cargo test -p ironflow-engine agent_resume`
//! selects them.

use std::future::pending;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use serde_json::json;
use tokio::spawn;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::error::AgentError;
use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
use ironflow_core::retry::RetryPolicy;
use ironflow_engine::config::{AgentStepConfig, DEFAULT_RESUME_PROMPT, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{WorkflowEvent, WorkflowEventBus};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{LeaseRequest, RunStatus, RunUpdate, Step, StepStatus, TriggerKind};
use ironflow_store::store::{RunStore, STEP_INTERRUPTED_ERROR, Store};

/// Test timeout for bodies that touch the store and spawn tasks.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name of every handler in this file.
const WORKFLOW: &str = "agent-resume";

/// Worker id holding the lease that is lost.
const WORKER: &str = "worker-that-dies";

/// Provider that hangs while `hang` is set, records every config it gets,
/// and can pretend that the session it is asked to resume does not exist.
struct ResumeProvider {
    hang: AtomicBool,
    sessions: bool,
    missing_session: bool,
    first_error: Mutex<Option<AgentError>>,
    calls: AtomicU32,
    seen: Mutex<Vec<AgentConfig>>,
}

impl ResumeProvider {
    fn new() -> Self {
        Self {
            hang: AtomicBool::new(true),
            sessions: true,
            missing_session: false,
            first_error: Mutex::new(None),
            calls: AtomicU32::new(0),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Every config the provider saw, in invocation order.
    fn seen(&self) -> Vec<AgentConfig> {
        self.seen.lock().expect("lock").clone()
    }
}

impl AgentProvider for ResumeProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            self.seen.lock().expect("lock").push(config.clone());
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.hang.load(Ordering::SeqCst) {
                pending::<()>().await;
            }
            if let Some(err) = self.first_error.lock().expect("lock").take() {
                return Err(err);
            }
            if let (true, Some(session_id)) = (self.missing_session, &config.resume_session_id) {
                return Err(AgentError::ProcessFailed {
                    exit_code: 1,
                    stderr: format!("No conversation found with session ID: {session_id}"),
                });
            }
            Ok(AgentOutput::new(json!("done")))
        })
    }

    fn supports_sessions_for(&self, _config: &AgentConfig) -> bool {
        self.sessions
    }
}

/// Runs the configured agent steps one after the other, or as one parallel
/// wave.
struct AgentWorkflow {
    steps: Vec<(&'static str, AgentStepConfig)>,
    parallel: bool,
}

impl AgentWorkflow {
    fn single(config: AgentStepConfig) -> Self {
        Self {
            steps: vec![("review", config)],
            parallel: false,
        }
    }
}

impl WorkflowHandler for AgentWorkflow {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            if self.parallel {
                let wave = self
                    .steps
                    .iter()
                    .map(|(name, config)| (*name, StepConfig::Agent(config.clone())))
                    .collect();
                ctx.parallel(wave, true).await?;
            } else {
                for (name, config) in &self.steps {
                    ctx.agent(name, config.clone()).await?;
                }
            }
            Ok(())
        })
    }
}

fn review_config() -> AgentStepConfig {
    AgentStepConfig::new("review the code").max_budget_usd(0.10)
}

fn engine_with(
    store: &Arc<InMemoryStore>,
    provider: &Arc<ResumeProvider>,
    handler: AgentWorkflow,
    bus: Option<WorkflowEventBus>,
) -> Arc<Engine> {
    let dyn_store: Arc<dyn Store> = store.clone();
    let dyn_provider: Arc<dyn AgentProvider> = provider.clone();
    let mut engine = Engine::new(dyn_store, dyn_provider);
    if let Some(bus) = bus {
        engine.set_event_bus(bus);
    }
    engine.register(handler).expect("register handler");
    Arc::new(engine)
}

async fn enqueue(engine: &Engine) -> Uuid {
    engine
        .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), 0)
        .await
        .expect("enqueue")
        .id
}

/// Pick the run the way a worker does, with a lease that expires at once.
async fn pick_with_expiring_lease(store: &InMemoryStore, run_id: Uuid) {
    let picked = store
        .pick_next_pending(Some(LeaseRequest {
            worker_id: WORKER.to_string(),
            ttl: Duration::from_millis(1),
        }))
        .await
        .expect("pick")
        .expect("a pending run");
    assert_eq!(picked.id, run_id);
}

/// Execute the run on a task and kill it once the provider has been invoked
/// `reached` times in total.
async fn crash_inside(engine: &Arc<Engine>, run_id: Uuid, provider: &ResumeProvider, reached: u32) {
    let worker = engine.clone();
    let task = spawn(async move { worker.execute_handler_run(run_id).await });
    while provider.calls.load(Ordering::SeqCst) < reached {
        sleep(Duration::from_millis(5)).await;
    }
    task.abort();
    let err = task.await.expect_err("the worker was killed");
    assert!(err.is_cancelled());
}

/// Reap the expired lease and interrupt the run's open steps like the reaper.
async fn recover(engine: &Engine, store: &InMemoryStore, run_id: Uuid) {
    sleep(Duration::from_millis(5)).await;
    let reaped = store
        .reap_expired_leases(100)
        .await
        .expect("reap")
        .into_iter()
        .find(|r| r.run.id == run_id)
        .expect("the run's lease expired");
    assert_eq!(reaped.to, RunStatus::Pending);
    engine
        .interrupt_running_steps(run_id)
        .await
        .expect("interrupt running steps");
}

/// Crash the run inside its agent steps, recover it, then run it to the end.
async fn crash_then_finish(
    engine: &Arc<Engine>,
    store: &InMemoryStore,
    provider: &ResumeProvider,
    agent_steps: u32,
) -> Uuid {
    let run_id = enqueue(engine).await;
    pick_with_expiring_lease(store, run_id).await;
    crash_inside(engine, run_id, provider, agent_steps).await;
    recover(engine, store, run_id).await;

    provider.hang.store(false, Ordering::SeqCst);
    pick_with_expiring_lease(store, run_id).await;
    engine
        .execute_handler_run(run_id)
        .await
        .expect("the requeued run completes");
    run_id
}

/// Steps of `run_id` called `name`, in creation order.
async fn steps_named(store: &InMemoryStore, run_id: Uuid, name: &str) -> Vec<Step> {
    let mut steps: Vec<Step> = store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .filter(|s| s.name == name)
        .collect();
    steps.sort_by_key(|s| s.created_at);
    steps
}

fn is_interrupted(step: &Step) -> bool {
    step.status.state == StepStatus::Failed && step.error.as_deref() == Some(STEP_INTERRUPTED_ERROR)
}

#[tokio::test]
async fn agent_resume_interrupted_step_resumes_its_session() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ResumeProvider::new());
        let bus = WorkflowEventBus::new();
        let engine = engine_with(
            &store,
            &provider,
            AgentWorkflow::single(review_config()),
            Some(bus.clone()),
        );

        let run_id = enqueue(&engine).await;
        let mut rx = bus.subscribe(run_id);
        pick_with_expiring_lease(&store, run_id).await;
        crash_inside(&engine, run_id, &provider, 1).await;
        recover(&engine, &store, run_id).await;

        let interrupted = steps_named(&store, run_id, "review").await;
        assert_eq!(interrupted.len(), 1);
        assert!(
            is_interrupted(&interrupted[0]),
            "step: {:?}",
            interrupted[0]
        );
        let session_id = interrupted[0]
            .session_id
            .clone()
            .expect("the session is recorded before the agent launches");
        Uuid::parse_str(&session_id).expect("the session id is a uuid");

        provider.hang.store(false, Ordering::SeqCst);
        pick_with_expiring_lease(&store, run_id).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the requeued run completes");

        let seen = provider.seen();
        assert_eq!(seen.len(), 2, "configs: {seen:?}");
        assert_eq!(seen[0].prompt, "review the code");
        assert_eq!(seen[0].session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(seen[0].resume_session_id, None);
        assert_eq!(seen[1].prompt, DEFAULT_RESUME_PROMPT);
        assert_eq!(
            seen[1].resume_session_id.as_deref(),
            Some(session_id.as_str())
        );

        let records = steps_named(&store, run_id, "review").await;
        assert_eq!(records.len(), 2, "steps: {records:?}");
        assert_eq!(records[1].status.state, StepStatus::Completed);
        assert_eq!(
            records[1].session_id.as_deref(),
            Some(session_id.as_str()),
            "the resumed step keeps the session it resumed"
        );

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);

        let mut resumed = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let WorkflowEvent::AgentStepResumed(payload) = event {
                resumed.push(payload);
            }
        }
        assert_eq!(resumed.len(), 1, "events: {resumed:?}");
        assert_eq!(resumed[0].step_name, "review");
        assert_eq!(resumed[0].step_index, records[1].position);
        assert_eq!(resumed[0].session_id, session_id);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn agent_resume_uses_the_step_resume_prompt() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ResumeProvider::new());
        let config = review_config().resume_prompt("Pick the review up and finish it.");
        let engine = engine_with(&store, &provider, AgentWorkflow::single(config), None);

        crash_then_finish(&engine, &store, &provider, 1).await;

        let seen = provider.seen();
        assert_eq!(seen.len(), 2, "configs: {seen:?}");
        assert_eq!(seen[0].prompt, "review the code");
        assert_eq!(seen[1].prompt, "Pick the review up and finish it.");
        assert_eq!(seen[1].resume_session_id, seen[0].session_id);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn agent_resume_restarts_from_scratch_when_the_session_is_gone() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ResumeProvider {
            missing_session: true,
            ..ResumeProvider::new()
        });
        let engine = engine_with(
            &store,
            &provider,
            AgentWorkflow::single(review_config()),
            None,
        );

        let run_id = crash_then_finish(&engine, &store, &provider, 1).await;

        let seen = provider.seen();
        assert_eq!(seen.len(), 3, "configs: {seen:?}");
        let session_id = seen[0].session_id.clone().expect("a pinned session");
        assert_eq!(
            seen[1].resume_session_id.as_deref(),
            Some(session_id.as_str())
        );
        assert_eq!(seen[2].prompt, "review the code");
        assert_eq!(seen[2].resume_session_id, None);
        assert_eq!(seen[2].session_id.as_deref(), Some(session_id.as_str()));

        let records = steps_named(&store, run_id, "review").await;
        assert_eq!(records.len(), 2, "steps: {records:?}");
        assert_eq!(records[1].status.state, StepStatus::Completed);
        assert_eq!(records[1].session_id.as_deref(), Some(session_id.as_str()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn agent_resume_is_skipped_for_a_provider_without_sessions() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ResumeProvider {
            sessions: false,
            ..ResumeProvider::new()
        });
        let engine = engine_with(
            &store,
            &provider,
            AgentWorkflow::single(review_config()),
            None,
        );

        let run_id = crash_then_finish(&engine, &store, &provider, 1).await;

        let seen = provider.seen();
        assert_eq!(seen.len(), 2, "configs: {seen:?}");
        for config in &seen {
            assert_eq!(config.prompt, "review the code");
            assert_eq!(config.session_id, None);
            assert_eq!(config.resume_session_id, None);
        }

        let records = steps_named(&store, run_id, "review").await;
        assert_eq!(records.len(), 2, "steps: {records:?}");
        assert!(records.iter().all(|s| s.session_id.is_none()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn agent_resume_step_retry_stays_in_the_session_of_the_step() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ResumeProvider {
            hang: AtomicBool::new(false),
            first_error: Mutex::new(Some(AgentError::ProcessFailed {
                exit_code: 1,
                stderr: "transient failure".to_string(),
            })),
            ..ResumeProvider::new()
        });
        let config =
            review_config().retry_policy(RetryPolicy::new(1).backoff(Duration::from_millis(1)));
        let engine = engine_with(&store, &provider, AgentWorkflow::single(config), None);

        let run_id = enqueue(&engine).await;
        pick_with_expiring_lease(&store, run_id).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the retried step completes");

        let seen = provider.seen();
        assert_eq!(seen.len(), 2, "configs: {seen:?}");
        let pinned = seen[0].session_id.clone().expect("a pinned session");
        assert_eq!(seen[0].resume_session_id, None);
        // The CLI refuses to create a session id twice: the retry resumes it.
        assert_eq!(seen[1].session_id, None);
        assert_eq!(seen[1].resume_session_id.as_deref(), Some(pinned.as_str()));
        assert!(seen.iter().all(|c| c.prompt == "review the code"));

        let records = steps_named(&store, run_id, "review").await;
        assert_eq!(records.len(), 1, "steps: {records:?}");
        assert_eq!(records[0].status.state, StepStatus::Completed);
        assert_eq!(records[0].session_id.as_deref(), Some(pinned.as_str()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn agent_resume_is_skipped_when_the_step_failed_and_the_run_is_retried() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ResumeProvider {
            hang: AtomicBool::new(false),
            first_error: Mutex::new(Some(AgentError::ProcessFailed {
                exit_code: 1,
                stderr: "agent crashed".to_string(),
            })),
            ..ResumeProvider::new()
        });
        let engine = engine_with(
            &store,
            &provider,
            AgentWorkflow::single(review_config()),
            None,
        );

        let run_id = engine
            .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), 1)
            .await
            .expect("enqueue")
            .id;
        store
            .pick_next_pending(None)
            .await
            .expect("pick")
            .expect("a pending run");
        engine
            .execute_handler_run(run_id)
            .await
            .expect_err("the agent step fails");
        let run = store.get_run(run_id).await.expect("get run").expect("run");
        assert_eq!(run.status.state, RunStatus::Retrying);

        // The backoff elapsed: the worker picks the retried run up.
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
        store
            .pick_next_pending(None)
            .await
            .expect("pick")
            .expect("the retried run");
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the retried run completes");

        let seen = provider.seen();
        assert_eq!(seen.len(), 2, "configs: {seen:?}");
        assert!(seen.iter().all(|c| c.resume_session_id.is_none()));
        assert!(seen.iter().all(|c| c.prompt == "review the code"));
        let failed = seen[0].session_id.clone().expect("a pinned session");
        let retried = seen[1].session_id.clone().expect("a pinned session");
        assert_ne!(failed, retried);

        let records = steps_named(&store, run_id, "review").await;
        assert_eq!(records.len(), 2, "steps: {records:?}");
        assert_eq!(records[0].status.state, StepStatus::Failed);
        assert!(!is_interrupted(&records[0]), "steps: {records:?}");
        assert_eq!(records[0].session_id.as_deref(), Some(failed.as_str()));
        assert_eq!(records[1].status.state, StepStatus::Completed);
        assert_eq!(records[1].session_id.as_deref(), Some(retried.as_str()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn agent_resume_step_set_by_the_author_is_left_alone() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ResumeProvider {
            hang: AtomicBool::new(false),
            ..ResumeProvider::new()
        });
        let own = "0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d";
        let config = review_config().resume(own);
        let engine = engine_with(&store, &provider, AgentWorkflow::single(config), None);

        let run_id = enqueue(&engine).await;
        pick_with_expiring_lease(&store, run_id).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the run completes");

        let seen = provider.seen();
        assert_eq!(seen.len(), 1, "configs: {seen:?}");
        assert_eq!(seen[0].resume_session_id.as_deref(), Some(own));
        assert_eq!(seen[0].session_id, None);
        assert_eq!(seen[0].prompt, "review the code");

        let records = steps_named(&store, run_id, "review").await;
        assert_eq!(records.len(), 1, "steps: {records:?}");
        assert_eq!(records[0].session_id, None);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn agent_resume_parallel_wave_resumes_each_step_session() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ResumeProvider::new());
        let handler = AgentWorkflow {
            steps: vec![
                (
                    "lint",
                    AgentStepConfig::new("lint the code").max_budget_usd(0.10),
                ),
                (
                    "test",
                    AgentStepConfig::new("test the code").max_budget_usd(0.10),
                ),
            ],
            parallel: true,
        };
        let engine = engine_with(&store, &provider, handler, None);

        let run_id = crash_then_finish(&engine, &store, &provider, 2).await;

        let seen = provider.seen();
        assert_eq!(seen.len(), 4, "configs: {seen:?}");
        for (name, prompt) in [("lint", "lint the code"), ("test", "test the code")] {
            let launched: Vec<&AgentConfig> = seen.iter().filter(|c| c.prompt == prompt).collect();
            assert_eq!(launched.len(), 1, "{name}: {seen:?}");
            let session_id = launched[0].session_id.clone().expect("a pinned session");

            let resumed: Vec<&AgentConfig> = seen
                .iter()
                .filter(|c| c.resume_session_id.as_deref() == Some(session_id.as_str()))
                .collect();
            assert_eq!(resumed.len(), 1, "{name}: {seen:?}");
            assert_eq!(resumed[0].prompt, DEFAULT_RESUME_PROMPT);

            let records = steps_named(&store, run_id, name).await;
            assert_eq!(records.len(), 2, "{name}: {records:?}");
            assert!(is_interrupted(&records[0]), "{name}: {records:?}");
            assert_eq!(records[1].status.state, StepStatus::Completed);
            assert_eq!(records[1].session_id.as_deref(), Some(session_id.as_str()));
        }
    })
    .await
    .expect("test timed out");
}
