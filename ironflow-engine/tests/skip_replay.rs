//! Non-regression test: `ctx.skip()` must replay its `Skipped` step on resume
//! instead of recording a second one.
//!
//! Drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`], the way `ironflow-engine/tests/human_input.rs` does.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::HumanInputConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`SkipWorkflow`].
const WORKFLOW: &str = "skip-replay";

/// The typed answer the handler asks for after the skip.
#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// Skips a step, then asks for [`Answers`].
struct SkipWorkflow;

impl WorkflowHandler for SkipWorkflow {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.skip("maybe", "not needed").await?;
            let answers = ctx
                .human_input::<Answers>("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            assert_eq!(answers.answers, vec!["ok".to_string()]);
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
        .expect("handler suspends on the input");
    run.id
}

#[tokio::test]
async fn skip_replay_does_not_create_a_second_skipped_step_on_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), SkipWorkflow);

        let run_id = start(&engine, &store).await;

        let steps = store.list_steps(run_id).await.expect("list steps");
        let skipped: Vec<_> = steps.iter().filter(|s| s.name == "maybe").collect();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].status.state, StepStatus::Skipped);
        let skipped_id = skipped[0].id;

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

        // Reproduces the worker / `ExecutionMode::Workers` resume path, not
        // `resume_run`.
        engine
            .execute_handler_run(run_id)
            .await
            .expect("resume via worker pickup");

        let steps = store.list_steps(run_id).await.expect("list steps");
        let skipped: Vec<_> = steps.iter().filter(|s| s.name == "maybe").collect();
        assert_eq!(skipped.len(), 1, "no second 'maybe' step must be created");
        assert_eq!(skipped[0].id, skipped_id);
        assert_eq!(skipped[0].status.state, StepStatus::Skipped);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}
