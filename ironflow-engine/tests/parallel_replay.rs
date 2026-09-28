//! Non-regression test: `ctx.parallel()` must replay a completed wave on
//! resume instead of launching its steps again.
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
use ironflow_engine::config::{HumanInputConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`ParallelWorkflow`].
const WORKFLOW: &str = "parallel-replay";

/// The typed answer the handler asks for after the wave.
#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// Runs a parallel wave of two shell steps, then asks for [`Answers`].
struct ParallelWorkflow;

impl WorkflowHandler for ParallelWorkflow {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.parallel(
                vec![
                    ("wave-a", StepConfig::Shell(ShellConfig::new("echo a"))),
                    ("wave-b", StepConfig::Shell(ShellConfig::new("echo b"))),
                ],
                true,
            )
            .await?;
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
async fn parallel_replay_does_not_rerun_a_completed_wave_on_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let handler = ParallelWorkflow;
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store).await;

        let steps = store.list_steps(run_id).await.expect("list steps");
        let wave_a = steps.iter().find(|s| s.name == "wave-a").expect("wave-a");
        let wave_b = steps.iter().find(|s| s.name == "wave-b").expect("wave-b");
        assert_eq!(wave_a.position, 0);
        assert_eq!(wave_b.position, 0);
        let (wave_a_id, wave_b_id) = (wave_a.id, wave_b.id);

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
        let wave_a_steps: Vec<_> = steps.iter().filter(|s| s.name == "wave-a").collect();
        let wave_b_steps: Vec<_> = steps.iter().filter(|s| s.name == "wave-b").collect();
        assert_eq!(
            wave_a_steps.len(),
            1,
            "no second 'wave-a' step must be created"
        );
        assert_eq!(
            wave_b_steps.len(),
            1,
            "no second 'wave-b' step must be created"
        );
        assert_eq!(wave_a_steps[0].id, wave_a_id);
        assert_eq!(wave_b_steps[0].id, wave_b_id);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}
