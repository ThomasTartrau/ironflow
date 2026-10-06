//! A step whose provider answers `CapacityWait` puts the run to sleep instead
//! of failing it, and re-executes from zero once the run wakes.
//!
//! A real provider plays the role of the account-aware provider: it answers
//! the configured capacity error on its first invocation, succeeds after
//! that, and records every config it receives so the tests assert on what
//! the re-executed step was given.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
use ironflow_engine::config::{AgentStepConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::wake::RunWaker;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, ProviderKind, RunStatus, RunUpdate, StepStatus, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store or spawn processes.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`ReviewWorkflow`].
const WORKFLOW: &str = "capacity-review";

/// Provider kind carried by the capacity errors.
const KIND: &str = "claude_subscription";

/// Error message the engine stores on a step parked for capacity.
const PARKED_ERROR: &str = "waiting for provider capacity";

/// Provider answering `first` on its first invocation, then succeeding.
struct CapacityProvider {
    first: Mutex<Option<AgentError>>,
    seen: Mutex<Vec<AgentConfig>>,
}

impl CapacityProvider {
    fn new(first: AgentError) -> Self {
        Self {
            first: Mutex::new(Some(first)),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Every config the provider saw, in invocation order.
    fn seen(&self) -> Vec<AgentConfig> {
        self.seen.lock().expect("lock").clone()
    }
}

impl AgentProvider for CapacityProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            self.seen.lock().expect("lock").push(config.clone());
            match self.first.lock().expect("lock").take() {
                Some(err) => Err(err),
                None => Ok(AgentOutput::new(json!("reviewed"))),
            }
        })
    }
}

/// Runs `before`, the `review` agent step, then `after`.
struct ReviewWorkflow {
    allow_failure: bool,
}

impl WorkflowHandler for ReviewWorkflow {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("before", ShellConfig::new("echo before")).await?;
            let mut review = AgentStepConfig::new("review").max_budget_usd(0.10);
            if self.allow_failure {
                review = review.allow_failure();
            }
            ctx.agent("review", review).await?;
            ctx.shell("after", ShellConfig::new("echo after")).await?;
            Ok(())
        })
    }
}

fn capacity_wait(wake_at: DateTime<Utc>) -> AgentError {
    AgentError::CapacityWait {
        kind: KIND.to_string(),
        wake_at,
    }
}

fn engine_with(
    store: Arc<InMemoryStore>,
    provider: Arc<CapacityProvider>,
    allow_failure: bool,
) -> Arc<Engine> {
    let dyn_store: Arc<dyn Store> = store;
    let dyn_provider: Arc<dyn AgentProvider> = provider;
    let mut engine = Engine::new(dyn_store, dyn_provider);
    engine
        .register(ReviewWorkflow { allow_failure })
        .expect("register handler");
    Arc::new(engine)
}

async fn wait_for_status(engine: &Engine, run_id: Uuid, status: RunStatus) {
    timeout(TEST_TIMEOUT, async {
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

#[tokio::test]
async fn capacity_wait_puts_the_run_to_sleep_until_wake_at() {
    timeout(TEST_TIMEOUT, async {
        let wake_at = Utc::now() + TimeDelta::hours(1);
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(CapacityProvider::new(capacity_wait(wake_at)));
        let engine = engine_with(store, provider, false);

        let result = engine
            .run_handler(WORKFLOW, TriggerKind::Api, json!({}))
            .await
            .expect("a capacity wait is a suspension, not a failure");

        let run = result.run;
        assert_eq!(run.status.state, RunStatus::Sleeping);
        assert_eq!(run.scheduled_at, Some(wake_at));
        assert_eq!(run.capacity_wait_kind, Some(ProviderKind::from(KIND)));

        let steps = engine.store().list_steps(run.id).await.unwrap();
        let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["before", "review"]);
        assert_eq!(steps[1].status.state, StepStatus::Failed);
        assert_eq!(steps[1].error.as_deref(), Some(PARKED_ERROR));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn woken_run_re_executes_the_step_and_completes() {
    timeout(TEST_TIMEOUT, async {
        let wake_at = Utc::now() + TimeDelta::hours(1);
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(CapacityProvider::new(capacity_wait(wake_at)));
        let engine = engine_with(store, provider.clone(), false);

        let result = engine
            .run_handler(WORKFLOW, TriggerKind::Api, json!({}))
            .await
            .unwrap();
        let run_id = result.run.id;
        engine
            .store()
            .update_run(
                run_id,
                RunUpdate {
                    scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let woken = RunWaker::new(engine.clone()).tick().await.unwrap();
        assert_eq!(woken.iter().map(|r| r.id).collect::<Vec<_>>(), vec![run_id]);
        wait_for_status(&engine, run_id, RunStatus::Completed).await;

        let run = engine.store().get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.capacity_wait_kind, None);

        let steps = engine.store().list_steps(run_id).await.unwrap();
        let reviews: Vec<_> = steps.iter().filter(|s| s.name == "review").collect();
        assert_eq!(
            reviews.len(),
            2,
            "the parked step is re-executed, not replayed"
        );
        let parked = reviews
            .iter()
            .find(|s| s.status.state == StepStatus::Failed)
            .expect("parked step kept");
        assert_eq!(parked.error.as_deref(), Some(PARKED_ERROR));
        assert!(
            reviews
                .iter()
                .any(|s| s.status.state == StepStatus::Completed)
        );
        assert_eq!(steps.iter().filter(|s| s.name == "before").count(), 1);
        assert_eq!(steps.iter().filter(|s| s.name == "after").count(), 1);

        // The re-executed step carries when the wait started, so the account
        // provider bounds the cumulative wait instead of restarting it.
        let seen = provider.seen();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].capacity_wait_since, None);
        assert_eq!(
            seen[1].capacity_wait_since,
            Some(parked.started_at.unwrap_or(parked.created_at))
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn allow_failure_does_not_swallow_a_capacity_wait() {
    timeout(TEST_TIMEOUT, async {
        let wake_at = Utc::now() + TimeDelta::hours(1);
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(CapacityProvider::new(capacity_wait(wake_at)));
        let engine = engine_with(store, provider, true);

        let result = engine
            .run_handler(WORKFLOW, TriggerKind::Api, json!({}))
            .await
            .unwrap();

        assert_eq!(result.run.status.state, RunStatus::Sleeping);
        let steps = engine.store().list_steps(result.run.id).await.unwrap();
        assert!(steps.iter().all(|s| s.name != "after"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn no_capacity_fails_the_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(CapacityProvider::new(AgentError::NoCapacity {
            kind: KIND.to_string(),
            next_reset: None,
        }));
        let engine = engine_with(store.clone(), provider, false);

        let run_id = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: WORKFLOW.to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run()
            .id;
        store.pick_next_pending(None).await.unwrap().unwrap();

        let err = engine
            .execute_handler_run(run_id)
            .await
            .expect_err("no capacity fails the step");
        assert!(matches!(
            err,
            EngineError::Operation(OperationError::Agent(AgentError::NoCapacity { .. }))
        ));

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.capacity_wait_kind, None);
    })
    .await
    .expect("test timed out");
}
