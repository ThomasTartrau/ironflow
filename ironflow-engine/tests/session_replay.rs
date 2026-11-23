//! An agent step hands out the `session_id` of its Claude Code session, that
//! a later step of the same run resumes, and that id survives a suspension:
//! the step replayed on resume returns the id recorded with it.
//!
//! A real provider plays the role of the Claude Code CLI: it records every
//! config it receives so the tests assert on the session each step ran in.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

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

/// Workflow name registered by [`ImplementThenFix`].
const WORKFLOW: &str = "implement-then-fix";

/// Session pinned by the author of the `pinned` workflow.
const PINNED: &str = "0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d";

/// Provider keeping every config it saw.
#[derive(Default)]
struct SessionProvider {
    seen: Mutex<Vec<AgentConfig>>,
}

impl AgentProvider for SessionProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            self.seen.lock().expect("lock").push(config.clone());
            Ok(AgentOutput::new(json!("ok")))
        })
    }
}

impl SessionProvider {
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

/// Implements, suspends on a human input, then resumes the implementation
/// session in a second agent step once the run is resumed.
struct ImplementThenFix {
    name: &'static str,
    pinned: Option<&'static str>,
}

impl WorkflowHandler for ImplementThenFix {
    fn name(&self) -> &str {
        self.name
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let mut implement = step("implement");
            if let Some(pinned) = self.pinned {
                implement = implement.session_id(pinned);
            }
            let reply = ctx.agent_with_meta("implement", implement).await?;
            let session = reply
                .session_id
                .expect("the implement step hands out its session id");
            ctx.human_input::<Answers>("review", HumanInputConfig::new("Comments?"))
                .await?;
            let fix = ctx.agent("fix", step("fix").resume(&session)).await?;
            assert_eq!(fix.session_id.as_deref(), Some(session.as_str()));
            Ok(())
        })
    }
}

fn step(prompt: &str) -> AgentStepConfig {
    AgentStepConfig::new(prompt).max_budget_usd(0.10)
}

/// Run `handler` up to its human input, answer it, and run it to the end.
async fn run_with_suspension(
    handler: ImplementThenFix,
) -> (Arc<InMemoryStore>, Arc<SessionProvider>, Uuid) {
    let store = Arc::new(InMemoryStore::new());
    let provider = Arc::new(SessionProvider::default());
    let dyn_store: Arc<dyn Store> = store.clone();
    let dyn_provider: Arc<dyn AgentProvider> = provider.clone();
    let name = handler.name;
    let mut engine = Engine::new(dyn_store, dyn_provider);
    engine.register(handler).expect("register handler");

    let run = engine
        .enqueue_handler_with_options(name, TriggerKind::Api, json!({}), EnqueueOptions::default())
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

    let stored = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(stored.status.state, RunStatus::Completed);
    (store, provider, run.id)
}

#[tokio::test]
async fn replayed_step_returns_the_session_it_ran_in() {
    timeout(TEST_TIMEOUT, async {
        let handler = ImplementThenFix {
            name: WORKFLOW,
            pinned: None,
        };
        let (store, provider, run_id) = run_with_suspension(handler).await;

        // The implement step ran once, in the session the engine pinned.
        let implemented = provider.configs_for("implement");
        assert_eq!(
            implemented.len(),
            1,
            "implement is replayed, not re-invoked"
        );
        let session = implemented[0]
            .session_id
            .clone()
            .expect("the engine pins a session");

        // Replayed from the store, the step still handed out that session.
        let fixed = provider.configs_for("fix");
        assert_eq!(fixed.len(), 1);
        assert_eq!(
            fixed[0].resume_session_id.as_deref(),
            Some(session.as_str())
        );
        assert_eq!(fixed[0].prompt, "fix");

        // The fix step records the session it resumed.
        let steps = store.list_steps(run_id).await.expect("list steps");
        let fix = steps.iter().find(|s| s.name == "fix").expect("fix stored");
        assert_eq!(fix.session_id.as_deref(), Some(session.as_str()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn step_with_an_author_session_hands_it_out_and_records_it() {
    timeout(TEST_TIMEOUT, async {
        let handler = ImplementThenFix {
            name: "pinned",
            pinned: Some(PINNED),
        };
        let (store, provider, run_id) = run_with_suspension(handler).await;

        let fixed = provider.configs_for("fix");
        assert_eq!(fixed[0].resume_session_id.as_deref(), Some(PINNED));
        let steps = store.list_steps(run_id).await.expect("list steps");
        let implement = steps
            .iter()
            .find(|s| s.name == "implement")
            .expect("implement stored");
        assert_eq!(implement.session_id.as_deref(), Some(PINNED));
    })
    .await
    .expect("test timed out");
}
