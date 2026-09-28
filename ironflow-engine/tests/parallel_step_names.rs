//! `ctx.parallel` rejects a wave where two branches share a step name.
//!
//! Branches of a wave share the run and the position: with the same name they
//! would share a trace id and the `ironflow.io/step` pod label, and the K8s
//! ephemeral provider would delete one branch's pod when starting the other.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::plan::PlanOptions;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunFilter, RunStatus, TriggerKind};
use ironflow_store::store::RunStore;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn engine_with(store: Arc<InMemoryStore>) -> Engine {
    let inner = ClaudeCodeProvider::new();
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ));
    Engine::new(store, provider)
}

struct DuplicateBranches;

impl WorkflowHandler for DuplicateBranches {
    fn name(&self) -> &str {
        "duplicate-branches"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.parallel(
                vec![
                    ("lint", StepConfig::Shell(ShellConfig::new("echo a"))),
                    ("test", StepConfig::Shell(ShellConfig::new("echo b"))),
                    ("lint", StepConfig::Shell(ShellConfig::new("echo c"))),
                ],
                true,
            )
            .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn parallel_duplicate_step_names_fail_before_any_step_is_created() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let mut engine = engine_with(store.clone());
        engine.register(DuplicateBranches).unwrap();

        let err = engine
            .run_handler("duplicate-branches", TriggerKind::Manual, json!({}))
            .await
            .expect_err("two branches named lint");
        let EngineError::InvalidWorkflow(message) = err else {
            panic!("expected InvalidWorkflow, got {err:?}");
        };
        assert!(message.contains("\"lint\""), "{message}");

        let runs = store
            .list_runs(RunFilter::default(), 1, 50)
            .await
            .expect("list runs");
        assert_eq!(runs.items.len(), 1);
        let run = &runs.items[0];
        assert_eq!(run.status.state, RunStatus::Failed);

        let steps = store.list_steps(run.id).await.expect("list steps");
        assert!(steps.is_empty(), "no step may be created: {steps:?}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_reports_duplicate_parallel_step_names() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(Arc::new(InMemoryStore::new()));
        engine.register(DuplicateBranches).unwrap();

        let plan = engine
            .plan_handler("duplicate-branches", json!({}), PlanOptions::default())
            .await
            .expect("a failing handler still yields a plan");

        assert!(plan.steps.is_empty(), "{:?}", plan.steps);
        assert!(plan.truncated);
        let reason = plan.incomplete_reason.expect("a reason");
        assert!(reason.contains("\"lint\""), "{reason}");
    })
    .await
    .expect("test timed out");
}
