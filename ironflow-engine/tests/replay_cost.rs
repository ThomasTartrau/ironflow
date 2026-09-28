//! Non-regression test: steps replayed on resume must not be charged twice.
//!
//! On resume, the context starts from the run cost persisted at suspension,
//! which already includes every completed step. Replaying a step, alone or in
//! a `parallel` wave, must not add its cost again, or the run reports money it
//! never spent and its cost cap refuses work it can afford.
//!
//! Agent steps replay recorded fixtures via [`RecordReplayProvider`], so every
//! step has a deterministic, known cost with no network or CLI involved.

use std::env::temp_dir;
use std::fs::{create_dir_all, remove_dir_all, write};
use std::process::id as process_id;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, to_string};
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::{RecordReplayProvider, hash_config};
use ironflow_engine::config::{AgentStepConfig, HumanInputConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, EnqueueOptions};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`AgentsAroundInput`].
const WORKFLOW: &str = "agents-around-input";

/// Declared budget of every agent step, in USD.
const STEP_BUDGET: f64 = 0.10;
/// Actual cost the replayed fixture reports for each agent step, in USD.
const STEP_COST: f64 = 0.10;

/// The typed answer the handler asks for.
#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// Runs one agent step and a wave of two, suspends on a human input, then
/// runs one more agent step once resumed.
struct AgentsAroundInput;

impl WorkflowHandler for AgentsAroundInput {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("solo", agent_config()).await?;
            ctx.parallel(
                vec![
                    ("wave-a", StepConfig::Agent(agent_config())),
                    ("wave-b", StepConfig::Agent(agent_config())),
                ],
                true,
            )
            .await?;
            let answers = ctx
                .human_input::<Answers>("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            assert_eq!(answers.answers, vec!["ok".to_string()]);
            ctx.agent("after", agent_config()).await?;
            Ok(())
        })
    }
}

/// Removes the fixtures directory when the test ends.
struct FixtureGuard(String);

impl Drop for FixtureGuard {
    fn drop(&mut self) {
        let _ = remove_dir_all(&self.0);
    }
}

/// The agent config used by every step, so a single fixture serves them all.
fn agent_config() -> AgentConfig {
    AgentStepConfig::new("summarize the build")
        .model("haiku")
        .max_budget_usd(STEP_BUDGET)
}

/// Write the replay fixture and return its directory plus a cleanup guard.
fn fixtures_dir() -> (String, FixtureGuard) {
    let dir = temp_dir()
        .join(format!(
            "ironflow-replay-cost-{}-{}",
            process_id(),
            Uuid::now_v7()
        ))
        .display()
        .to_string();
    let guard = FixtureGuard(dir.clone());

    let config = agent_config();
    let mut output = AgentOutput::new(json!("done"));
    output.cost_usd = Some(STEP_COST);
    output.duration_ms = 10;

    create_dir_all(&dir).expect("create fixtures dir");
    let fixture = json!({ "config": config, "output": output });
    write(
        format!("{dir}/{}.json", hash_config(&config)),
        to_string(&fixture).expect("serialize fixture"),
    )
    .expect("write fixture");

    (dir, guard)
}

#[tokio::test]
async fn replayed_steps_are_not_charged_twice_on_resume() {
    timeout(TEST_TIMEOUT, async {
        let (dir, _guard) = fixtures_dir();
        let store = Arc::new(InMemoryStore::new());
        let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
            ClaudeCodeProvider::new(),
            &dir,
        ));
        let dyn_store: Arc<dyn Store> = store.clone();
        let mut engine = Engine::new(dyn_store, provider);
        engine
            .register(AgentsAroundInput)
            .expect("register handler");

        // Four agent steps at $0.10 each: the cap is met exactly, so a single
        // replayed step charged twice makes the last one cross it.
        let cap = Decimal::new(40, 2);
        let run = engine
            .enqueue_handler_with_options(
                WORKFLOW,
                TriggerKind::Api,
                json!({}),
                EnqueueOptions {
                    max_cost_usd: Some(cap),
                    ..Default::default()
                },
            )
            .await
            .expect("enqueue")
            .into_run();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .expect("mark running");
        engine
            .execute_handler_run(run.id)
            .await
            .expect("handler suspends on the input");

        let suspended = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(suspended.cost_usd, Decimal::new(30, 2));

        let steps = store.list_steps(run.id).await.expect("list steps");
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
            .update_run_status(run.id, RunStatus::Running)
            .await
            .expect("mark running");

        // Reproduces the worker / `ExecutionMode::Workers` resume path.
        engine
            .execute_handler_run(run.id)
            .await
            .expect("the last step fits the cap once replays are not charged");

        let run = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
        assert_eq!(run.cost_usd, cap, "each agent step is charged exactly once");
    })
    .await
    .expect("test timed out");
}
