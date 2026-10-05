//! An agent step hands out an `environment_id` that a later step resumes,
//! and that id survives a suspension: the step replayed on resume returns the
//! id persisted with it instead of calling the provider again.
//!
//! A real provider plays the role of the K8s ephemeral provider: it hands out
//! a fresh id when no resume id is set, echoes the resume id otherwise, and
//! records every config it receives so the tests assert on what it saw.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::time::timeout;

use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
use ironflow_engine::config::{AgentStepConfig, HumanInputConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, EnqueueOptions};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`PrepareThenContinue`].
const WORKFLOW: &str = "prepare-then-continue";

/// Environment id the provider hands out to the first fresh invocation.
const FIRST_ENVIRONMENT: &str = "ironflow-env-1";

/// Provider handing out environment ids and keeping every config it saw.
#[derive(Default)]
struct EnvironmentProvider {
    seen: Mutex<Vec<AgentConfig>>,
}

impl AgentProvider for EnvironmentProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            let mut seen = self.seen.lock().expect("lock");
            seen.push(config.clone());
            let fresh = seen
                .iter()
                .filter(|c| c.resume_environment_id.is_none())
                .count();
            let mut output = AgentOutput::new(json!("ok"));
            output.environment_id = Some(
                config
                    .resume_environment_id
                    .clone()
                    .unwrap_or_else(|| format!("ironflow-env-{fresh}")),
            );
            Ok(output)
        })
    }
}

impl EnvironmentProvider {
    /// Every config the provider saw for `prompt`, in invocation order.
    fn configs_for(&self, prompt: &str) -> Vec<AgentConfig> {
        let seen = self.seen.lock().expect("lock");
        seen.iter()
            .filter(|c| c.prompt == prompt)
            .cloned()
            .collect()
    }
}

/// The typed answer the handler asks for; only its schema is used.
#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct Answers {
    answers: Vec<String>,
}

/// Prepares an environment, suspends on a human input, then resumes the
/// environment in a second agent step once the run is resumed.
struct PrepareThenContinue;

impl WorkflowHandler for PrepareThenContinue {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let prepared = ctx.agent("prepare", step("prepare")).await?;
            let environment = prepared
                .environment_id
                .expect("the prepare step hands out an environment id");
            ctx.human_input::<Answers>("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            ctx.agent(
                "continue",
                step("continue").resume_environment(&environment),
            )
            .await?;
            Ok(())
        })
    }
}

fn step(prompt: &str) -> AgentStepConfig {
    AgentStepConfig::new(prompt).max_budget_usd(0.10)
}

#[tokio::test]
async fn replayed_step_returns_its_persisted_environment_id() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(EnvironmentProvider::default());
        let dyn_store: Arc<dyn Store> = store.clone();
        let dyn_provider: Arc<dyn AgentProvider> = provider.clone();
        let mut engine = Engine::new(dyn_store, dyn_provider);
        engine
            .register(PrepareThenContinue)
            .expect("register handler");

        let run = engine
            .enqueue_handler_with_options(
                WORKFLOW,
                TriggerKind::Api,
                json!({}),
                EnqueueOptions::default(),
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

        let steps = store.list_steps(run.id).await.expect("list steps");
        let prepare = steps
            .iter()
            .find(|s| s.name == "prepare")
            .expect("prepare step stored");
        assert_eq!(prepare.environment_id.as_deref(), Some(FIRST_ENVIRONMENT));

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

        engine
            .execute_handler_run(run.id)
            .await
            .expect("the run completes once resumed");

        let run = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);

        // The prepare step was replayed from the store, not invoked again,
        // yet the continue step still received the id it handed out.
        let prepared = provider.configs_for("prepare");
        assert_eq!(prepared.len(), 1, "prepare is replayed, not re-invoked");
        assert_eq!(prepared[0].resume_environment_id, None);

        let continued = provider.configs_for("continue");
        assert_eq!(continued.len(), 1);
        assert_eq!(
            continued[0].resume_environment_id.as_deref(),
            Some(FIRST_ENVIRONMENT)
        );

        let steps = store.list_steps(run.id).await.expect("list steps");
        let continue_step = steps
            .iter()
            .find(|s| s.name == "continue")
            .expect("continue step stored");
        assert_eq!(
            continue_step.environment_id.as_deref(),
            Some(FIRST_ENVIRONMENT)
        );
    })
    .await
    .expect("test timed out");
}
