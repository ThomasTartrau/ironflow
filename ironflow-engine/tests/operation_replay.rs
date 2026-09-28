//! Non-regression test: `ctx.operation()` must replay a completed step on
//! resume instead of calling `Operation::execute` again.
//!
//! Drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`], the way `ironflow-engine/tests/human_input.rs` does.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::error::OperationError;
use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::HumanInputConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::operation::{Operation, OperationContext};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`OperationWorkflow`].
const WORKFLOW: &str = "operation-replay";

/// The typed answer the handler asks for after the operation step.
#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// A custom operation that counts how many times it actually ran.
struct CountingOp {
    calls: Arc<AtomicU32>,
}

#[async_trait]
impl Operation for CountingOp {
    fn kind(&self) -> &str {
        "counting-op"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"called": true}))
    }
}

/// Runs a custom operation, then asks for [`Answers`].
struct OperationWorkflow {
    op: CountingOp,
}

impl WorkflowHandler for OperationWorkflow {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("op", &self.op).await?;
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
async fn operation_replay_does_not_rerun_a_completed_operation_step_on_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicU32::new(0));
        let handler = OperationWorkflow {
            op: CountingOp {
                calls: calls.clone(),
            },
        };
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let steps = store.list_steps(run_id).await.expect("list steps");
        let op_steps: Vec<_> = steps.iter().filter(|s| s.name == "op").collect();
        assert_eq!(op_steps.len(), 1);
        let op_step_id = op_steps[0].id;

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

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the operation must not be re-executed on resume"
        );

        let steps = store.list_steps(run_id).await.expect("list steps");
        let op_steps: Vec<_> = steps.iter().filter(|s| s.name == "op").collect();
        assert_eq!(op_steps.len(), 1, "no second 'op' step must be created");
        assert_eq!(op_steps[0].id, op_step_id);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}
