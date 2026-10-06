//! Before every execution of a run, the engine asks the agent provider to
//! release what a previous execution left running (the K8s provider deletes
//! the run's pods), and a failed release fails the execution with a
//! replayable error.
//!
//! A real provider keeps a journal of the calls it receives; the tests
//! assert on what it saw and in which order.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::provider::{
    AgentConfig, AgentOutput, AgentProvider, InvokeFuture, LABEL_ROOT_RUN_ID, LABEL_RUN_ID,
    ReleaseFuture,
};
use ironflow_engine::config::{AgentStepConfig, ApprovalConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, RunStatus, RunUpdate, TriggerKind};
use ironflow_store::store::RunStore;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Provider answering "ok", journaling every release and invocation.
#[derive(Default)]
struct JournalProvider {
    journal: Mutex<Vec<String>>,
    configs: Mutex<Vec<AgentConfig>>,
    fail_release: AtomicBool,
}

impl JournalProvider {
    fn failing_release() -> Self {
        Self {
            fail_release: AtomicBool::new(true),
            ..Self::default()
        }
    }

    fn journal(&self) -> Vec<String> {
        self.journal.lock().expect("lock").clone()
    }

    /// The pod labels of the config whose prompt is `prompt`.
    fn labels_for(&self, prompt: &str) -> BTreeMap<String, String> {
        let configs = self.configs.lock().expect("lock");
        let config = configs.iter().find(|c| c.prompt == prompt);
        config.expect("provider saw the prompt").pod_labels.clone()
    }
}

impl AgentProvider for JournalProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            let entry = format!("invoke:{}", config.prompt);
            self.journal.lock().expect("lock").push(entry);
            self.configs.lock().expect("lock").push(config.clone());
            Ok(AgentOutput::new(json!("ok")))
        })
    }

    fn release_run<'a>(&'a self, run_id: &'a str) -> ReleaseFuture<'a> {
        Box::pin(async move {
            self.journal
                .lock()
                .expect("lock")
                .push(format!("release:{run_id}"));
            if self.fail_release.load(Ordering::SeqCst) {
                return Err(AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: "k8s api unreachable".to_string(),
                });
            }
            Ok(())
        })
    }
}

fn step(prompt: &str) -> AgentStepConfig {
    AgentStepConfig::new(prompt).max_budget_usd(0.10)
}

/// One agent step; fails transiently after it on the first `failures` runs.
struct Investigate {
    failures: u32,
    attempts: AtomicU32,
}

impl Investigate {
    fn new(failures: u32) -> Self {
        Self {
            failures,
            attempts: AtomicU32::new(0),
        }
    }
}

impl WorkflowHandler for Investigate {
    fn name(&self) -> &str {
        "investigate"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            ctx.agent("investigate", step(&format!("attempt-{attempt}")))
                .await?;
            if attempt < self.failures {
                return Err(EngineError::Operation(OperationError::Http {
                    status: Some(503),
                    message: "upstream unavailable".to_string(),
                }));
            }
            Ok(())
        })
    }
}

/// An agent step, an approval gate, then another agent step.
struct Gated;

impl WorkflowHandler for Gated {
    fn name(&self) -> &str {
        "gated"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("plan", step("before-gate")).await?;
            ctx.approval("gate", ApprovalConfig::new("Apply?")).await?;
            ctx.agent("apply", step("after-gate")).await?;
            Ok(())
        })
    }
}

/// Run [`Gated`] up to its gate, then approve it like the approval API does.
async fn approved_gated_run(engine: &Engine, store: &InMemoryStore) -> Uuid {
    let run_id = enqueue(store, "gated", 0).await;
    let suspended = engine.execute_run(run_id).await.expect("reaches the gate");
    assert_eq!(suspended.run.status.state, RunStatus::AwaitingApproval);
    store
        .update_run_status(run_id, RunStatus::Running)
        .await
        .expect("to running");
    run_id
}

/// Input of [`Child`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ChildInput {}

/// A sub-workflow with one agent step.
struct Child;

impl WorkflowHandler for Child {
    fn name(&self) -> &str {
        "child"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("implement", step("in-child")).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Child {
    type Input = ChildInput;
}

/// An agent step, then [`Child`]; keeps the child run id.
struct Parent {
    child_run_id: Arc<Mutex<Option<Uuid>>>,
}

impl WorkflowHandler for Parent {
    fn name(&self) -> &str {
        "parent"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("plan", step("in-parent")).await?;
            let child = ctx.workflow(&Child, ChildInput {}).await?;
            *self.child_run_id.lock().expect("lock") = Some(child.run_id());
            Ok(())
        })
    }
}

/// `(workflow, run_id, root_run_id)` as seen by each handler.
type SeenIds = Arc<Mutex<Vec<(&'static str, Uuid, Uuid)>>>;

/// A child that records the ids its context reports.
#[derive(Clone)]
struct RootChild {
    seen: SeenIds,
}

impl WorkflowHandler for RootChild {
    fn name(&self) -> &str {
        "root-child"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let ids = ("child", ctx.run_id(), ctx.root_run_id());
            self.seen.lock().expect("lock").push(ids);
            Ok(())
        })
    }
}

impl TypedWorkflow for RootChild {
    type Input = ChildInput;
}

/// Records its ids, then runs [`RootChild`].
struct RootParent {
    child: RootChild,
}

impl WorkflowHandler for RootParent {
    fn name(&self) -> &str {
        "root-parent"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let ids = ("parent", ctx.run_id(), ctx.root_run_id());
            self.child.seen.lock().expect("lock").push(ids);
            ctx.workflow(&self.child, ChildInput {}).await?;
            Ok(())
        })
    }
}

/// Create a run and pick it up like a worker would.
async fn enqueue(store: &InMemoryStore, workflow: &str, max_retries: u32) -> Uuid {
    let run_id = store
        .create_run(NewRun {
            created_by: None,
            workflow_name: workflow.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            concurrency_key: None,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("create run")
        .into_run()
        .id;
    let picked = store.pick_next_pending(None).await.expect("pick");
    assert_eq!(picked.expect("the new run").id, run_id);
    run_id
}

/// Pick a run armed for retry as if its backoff had elapsed.
async fn fast_forward_backoff(store: &InMemoryStore, run_id: Uuid) {
    let rewind = RunUpdate {
        scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
        ..RunUpdate::default()
    };
    store.update_run(run_id, rewind).await.expect("rewind");
    let picked = store.pick_next_pending(None).await.expect("pick");
    assert_eq!(picked.expect("a run waiting for its retry").id, run_id);
}

async fn status(store: &InMemoryStore, run_id: Uuid) -> RunStatus {
    store
        .get_run(run_id)
        .await
        .expect("get")
        .expect("run")
        .status
        .state
}

#[tokio::test]
async fn release_run_comes_before_the_first_step() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(JournalProvider::default());
        let mut engine = Engine::new(store.clone(), provider.clone());
        engine.register(Investigate::new(0)).expect("register");
        let run_id = enqueue(&store, "investigate", 0).await;

        engine.execute_run(run_id).await.expect("run completes");

        let expected = vec![format!("release:{run_id}"), "invoke:attempt-0".to_string()];
        assert_eq!(provider.journal(), expected);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn release_run_happens_on_every_execution() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(JournalProvider::default());
        let mut engine = Engine::new(store.clone(), provider.clone());
        engine.register(Investigate::new(1)).expect("register");
        let run_id = enqueue(&store, "investigate", 1).await;

        let first = engine.execute_run(run_id).await;
        assert!(first.is_err(), "first attempt fails transiently");
        assert_eq!(status(&store, run_id).await, RunStatus::Retrying);
        fast_forward_backoff(&store, run_id).await;
        engine.execute_run(run_id).await.expect("second attempt");

        let release = format!("release:{run_id}");
        let expected = vec![
            release.clone(),
            "invoke:attempt-0".to_string(),
            release,
            "invoke:attempt-1".to_string(),
        ];
        assert_eq!(provider.journal(), expected);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn failed_release_retries_the_run_without_running_it() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(JournalProvider::failing_release());
        let mut engine = Engine::new(store.clone(), provider.clone());
        engine.register(Investigate::new(0)).expect("register");
        let run_id = enqueue(&store, "investigate", 1).await;

        let err = engine.execute_run(run_id).await.expect_err("release fails");

        assert!(
            matches!(err, EngineError::Operation(OperationError::Agent(_))),
            "{err:?}"
        );
        assert_eq!(status(&store, run_id).await, RunStatus::Retrying);
        assert_eq!(provider.journal(), vec![format!("release:{run_id}")]);
        let steps = store.list_steps(run_id).await.expect("steps");
        assert!(steps.is_empty(), "no step may run: {steps:?}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn failed_release_without_retries_left_fails_the_run_with_its_cause() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(JournalProvider::failing_release());
        let mut engine = Engine::new(store.clone(), provider.clone());
        engine.register(Investigate::new(0)).expect("register");
        let run_id = enqueue(&store, "investigate", 0).await;

        let err = engine.execute_run(run_id).await.expect_err("release fails");

        assert!(!matches!(err, EngineError::InvalidWorkflow(_)), "{err:?}");
        let run = store.get_run(run_id).await.expect("get").expect("run");
        assert_eq!(run.status.state, RunStatus::Failed);
        let error = run.error.expect("error recorded");
        assert!(error.contains("k8s api unreachable"), "{error}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_run_releases_before_the_replay() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(JournalProvider::default());
        let mut engine = Engine::new(store.clone(), provider.clone());
        engine.register(Gated).expect("register");
        let run_id = approved_gated_run(&engine, &store).await;

        let resumed = engine.resume_run(run_id).await.expect("resume");

        assert_eq!(resumed.run.status.state, RunStatus::Completed);
        let release = format!("release:{run_id}");
        let expected = vec![
            release.clone(),
            "invoke:before-gate".to_string(),
            release,
            "invoke:after-gate".to_string(),
        ];
        assert_eq!(provider.journal(), expected);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn failed_release_on_resume_fails_the_run_before_the_next_step() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(JournalProvider::default());
        let mut engine = Engine::new(store.clone(), provider.clone());
        engine.register(Gated).expect("register");
        let run_id = approved_gated_run(&engine, &store).await;
        provider.fail_release.store(true, Ordering::SeqCst);

        let err = engine.resume_run(run_id).await.expect_err("release fails");

        assert!(
            matches!(err, EngineError::Operation(OperationError::Agent(_))),
            "{err:?}"
        );
        let run = store.get_run(run_id).await.expect("get").expect("run");
        assert_eq!(run.status.state, RunStatus::Failed);
        let error = run.error.expect("error recorded");
        assert!(error.contains("k8s api unreachable"), "{error}");
        let journal = provider.journal();
        assert!(
            !journal.contains(&"invoke:after-gate".to_string()),
            "{journal:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sub_workflow_agent_steps_carry_the_root_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(JournalProvider::default());
        let mut engine = Engine::new(store.clone(), provider.clone());
        let child_run_id = Arc::new(Mutex::new(None));
        engine
            .register(Parent {
                child_run_id: child_run_id.clone(),
            })
            .expect("register parent");
        engine.register(Child).expect("register child");
        let run_id = enqueue(&store, "parent", 0).await;

        engine.execute_run(run_id).await.expect("run completes");

        let root = run_id.to_string();
        let child = child_run_id
            .lock()
            .expect("lock")
            .expect("child ran")
            .to_string();
        let parent_labels = provider.labels_for("in-parent");
        assert_eq!(parent_labels[LABEL_RUN_ID], root);
        assert_eq!(parent_labels[LABEL_ROOT_RUN_ID], root);
        let child_labels = provider.labels_for("in-child");
        assert_eq!(child_labels[LABEL_RUN_ID], child);
        assert_eq!(child_labels[LABEL_ROOT_RUN_ID], root);
        // Only the top-level run is released: its root label covers the child.
        assert_eq!(provider.journal()[0], format!("release:{root}"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn root_run_id_is_the_top_level_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let mut engine = Engine::new(store.clone(), Arc::new(JournalProvider::default()));
        let child = RootChild {
            seen: SeenIds::default(),
        };
        engine
            .register(RootParent {
                child: child.clone(),
            })
            .expect("register parent");
        engine.register(child.clone()).expect("register child");
        let run_id = enqueue(&store, "root-parent", 0).await;

        engine.execute_run(run_id).await.expect("run completes");

        let seen = child.seen.lock().expect("lock").clone();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(seen[0], ("parent", run_id, run_id));
        let (who, child_run, child_root) = seen[1];
        assert_eq!(who, "child");
        assert_ne!(child_run, run_id);
        assert_eq!(child_root, run_id);
    })
    .await
    .expect("test timed out");
}
