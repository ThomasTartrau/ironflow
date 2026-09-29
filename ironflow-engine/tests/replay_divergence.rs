//! Non-regression tests: resuming a run whose handler changed while it was
//! suspended must fail with `EngineError::ReplayDivergence` (or
//! `EngineError::HandlerVersionMismatch`) instead of silently serving
//! another step's cached output, vote or approval.
//!
//! Drives real [`Engine`]s over a real [`InMemoryStore`] with real
//! [`WorkflowHandler`] implementations, the way
//! `ironflow-engine/tests/operation_replay.rs` and `skip_replay.rs` do. A
//! handler redeploy is simulated by registering a different handler on a
//! second [`Engine`] built over the same store.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::{HumanInputConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name shared by every handler in this file: a resume must find
/// the run under the name it was enqueued with.
const WORKFLOW: &str = "replay-divergence";

/// The typed answer the handler asks for after the suspending step.
///
/// The field itself is never read: what these tests assert on is that the
/// resume either replays or rejects the step that produces it, not the
/// answer's content.
#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct Answers {
    answers: Vec<String>,
}

/// The handler that starts the run: two shell steps, then a suspending
/// human input, then a step that must never run once resumed by a different
/// handler.
struct OldHandler {
    never_reached: Arc<AtomicU32>,
}

impl WorkflowHandler for OldHandler {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("kill-list-pods", ShellConfig::new("echo pods"))
                .await?;
            ctx.shell("clear-previous-attempt", ShellConfig::new("echo clear"))
                .await?;
            ctx.human_input::<Answers>("confirm", HumanInputConfig::new("go?"))
                .await?;
            self.never_reached.fetch_add(1, Ordering::SeqCst);
            ctx.shell("never-reached", ShellConfig::new("echo unreachable"))
                .await?;
            Ok(())
        })
    }
}

/// Simulates a redeploy of [`OldHandler`]: the first shell step is gone, so
/// what the old run recorded at position 0 ("kill-list-pods") no longer
/// matches what this handler calls there ("clear-previous-attempt").
struct NewHandler {
    never_reached: Arc<AtomicU32>,
}

impl WorkflowHandler for NewHandler {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("clear-previous-attempt", ShellConfig::new("echo clear"))
                .await?;
            ctx.human_input::<Answers>("confirm", HumanInputConfig::new("go?"))
                .await?;
            self.never_reached.fetch_add(1, Ordering::SeqCst);
            ctx.shell("never-reached", ShellConfig::new("echo unreachable"))
                .await?;
            Ok(())
        })
    }
}

/// A handler identical to [`OldHandler`] but pinned to version `"1.0.0"`,
/// with no declared compatible versions.
struct V1Handler;

impl WorkflowHandler for V1Handler {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn version(&self) -> Option<&str> {
        Some("1.0.0")
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.human_input::<Answers>("confirm", HumanInputConfig::new("go?"))
                .await?;
            Ok(())
        })
    }
}

/// Same body as [`V1Handler`], pinned to version `"2.0.0"` with no declared
/// compatible versions: incompatible with a run created by [`V1Handler`].
struct V2Handler;

impl WorkflowHandler for V2Handler {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn version(&self) -> Option<&str> {
        Some("2.0.0")
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.human_input::<Answers>("confirm", HumanInputConfig::new("go?"))
                .await?;
            Ok(())
        })
    }
}

/// Same body as [`V1Handler`], pinned to version `"2.0.0"` but declaring
/// `"1.0.0"` as a compatible version: compatible with a run created by
/// [`V1Handler`].
struct V2CompatHandler;

impl WorkflowHandler for V2CompatHandler {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn version(&self) -> Option<&str> {
        Some("2.0.0")
    }

    fn compatible_versions(&self) -> &[&str] {
        &["1.0.0"]
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.human_input::<Answers>("confirm", HumanInputConfig::new("go?"))
                .await?;
            Ok(())
        })
    }
}

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(ClaudeCodeProvider::new())
}

fn engine_with(store: Arc<InMemoryStore>, handler: impl WorkflowHandler + 'static) -> Engine {
    let store: Arc<dyn Store> = store;
    let mut engine = Engine::new(store, provider());
    engine.register(handler).expect("register handler");
    engine
}

/// Enqueue and execute a run the way the worker does, returning its id.
async fn start(engine: &Engine, store: &Arc<InMemoryStore>) -> Uuid {
    let run = engine
        .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), 0)
        .await
        .expect("enqueue");
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("mark running");
    engine
        .execute_handler_run(run.id)
        .await
        .expect("handler suspends on the human input");
    run.id
}

/// Mark the suspended `confirm` human input step answered, and the run
/// `Running` again, without executing anything: the caller drives the resume
/// itself (via a possibly different engine/handler).
async fn answer_confirm(store: &Arc<InMemoryStore>, run_id: Uuid) {
    let steps = store.list_steps(run_id).await.expect("list steps");
    let input_step = steps
        .iter()
        .find(|s| s.status.state == StepStatus::AwaitingApproval)
        .expect("human input step suspended the run");
    store
        .update_step(
            input_step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                output: Some(json!({"answers": ["ok"]})),
                completed_at: Some(Utc::now()),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await
        .expect("store the answer");
    store
        .update_run_status(run_id, RunStatus::Running)
        .await
        .expect("mark running");
}

#[tokio::test]
async fn resume_after_handler_removes_a_step_fails_with_replay_divergence() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let old_never_reached = Arc::new(AtomicU32::new(0));
        let engine1 = engine_with(
            store.clone(),
            OldHandler {
                never_reached: old_never_reached.clone(),
            },
        );

        let run_id = start(&engine1, &store).await;

        let steps = store.list_steps(run_id).await.expect("list steps");
        let completed: Vec<_> = steps
            .iter()
            .filter(|s| s.status.state == StepStatus::Completed)
            .collect();
        assert_eq!(completed.len(), 2, "the two shell steps must have run");
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);

        answer_confirm(&store, run_id).await;

        let new_never_reached = Arc::new(AtomicU32::new(0));
        let engine2 = engine_with(
            store.clone(),
            NewHandler {
                never_reached: new_never_reached.clone(),
            },
        );

        let err = engine2
            .execute_handler_run(run_id)
            .await
            .expect_err("a handler that dropped a step must fail the resume");

        assert!(
            matches!(err, EngineError::ReplayDivergence { position: 0, .. }),
            "unexpected error: {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("divergence"), "message was: {msg}");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);

        assert_eq!(old_never_reached.load(Ordering::SeqCst), 0);
        assert_eq!(new_never_reached.load(Ordering::SeqCst), 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_with_identical_handler_replays_unchanged() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let never_reached = Arc::new(AtomicU32::new(0));
        let engine = engine_with(
            store.clone(),
            OldHandler {
                never_reached: never_reached.clone(),
            },
        );

        let run_id = start(&engine, &store).await;
        answer_confirm(&store, run_id).await;

        engine
            .execute_handler_run(run_id)
            .await
            .expect("resume with the same handler must succeed");

        assert_eq!(never_reached.load(Ordering::SeqCst), 1);

        let steps = store.list_steps(run_id).await.expect("list steps");
        let shell_steps: Vec<_> = steps
            .iter()
            .filter(|s| s.name == "kill-list-pods" || s.name == "clear-previous-attempt")
            .collect();
        assert_eq!(shell_steps.len(), 2, "no step must be duplicated on resume");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_with_incompatible_handler_version_is_refused() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine1 = engine_with(store.clone(), V1Handler);

        let run_id = start(&engine1, &store).await;
        answer_confirm(&store, run_id).await;

        let steps_before = store.list_steps(run_id).await.expect("list steps").len();

        let engine2 = engine_with(store.clone(), V2Handler);
        let err = engine2
            .execute_handler_run(run_id)
            .await
            .expect_err("an incompatible handler version must refuse the resume");

        assert!(
            matches!(err, EngineError::HandlerVersionMismatch { .. }),
            "unexpected error: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("HANDLER_VERSION_MISMATCH"),
            "message was: {msg}"
        );
        assert!(msg.contains("1.0.0"), "message was: {msg}");
        assert!(msg.contains("2.0.0"), "message was: {msg}");

        let steps_after = store.list_steps(run_id).await.expect("list steps").len();
        assert_eq!(
            steps_before, steps_after,
            "no step must be created before the version check"
        );

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_with_compatible_handler_version_is_accepted() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine1 = engine_with(store.clone(), V1Handler);

        let run_id = start(&engine1, &store).await;
        answer_confirm(&store, run_id).await;

        let engine2 = engine_with(store.clone(), V2CompatHandler);
        engine2
            .execute_handler_run(run_id)
            .await
            .expect("a declared-compatible handler version must resume");

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}
