//! Integration tests for reusable agent roles.
//!
//! `ctx.agent("step", Role.task(input))` answers with the output type of the
//! role, applies the call-site overrides, and is persisted and replayed like
//! any other agent step. The agent provider is the harness mock: it plays the
//! model, the engine code path is real.

use std::env::temp_dir;
use std::fs::{create_dir_all, remove_dir_all, write};
use std::process::id as process_id;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, to_string, to_value};
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::operations::agent::{Effort, Model, PermissionMode};
use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
use ironflow_core::providers::record_replay::{RecordReplayProvider, hash_config};
use ironflow_engine::config::{
    AgentRole, AgentStep, AgentStepConfig, HumanInputConfig, StepConfig, Tool,
};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, EnqueueOptions};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::testing::{MissingAgentProvider, TestEngine};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`FixtureReview`].
const WORKFLOW: &str = "fixture-review";

/// What the reviewer role answers.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
struct Verdict {
    approved: bool,
    score: u8,
}

/// A reviewer with a persona, a read-only tool and a locked permission mode.
struct Reviewer;

impl AgentRole for Reviewer {
    type Input = String;
    type Output = Verdict;

    fn prompt(&self, input: String) -> String {
        format!("Review: {input}")
    }

    fn append_system_prompt(&self) -> Option<&str> {
        Some("You are a strict reviewer.")
    }

    fn tools(&self) -> Vec<Tool> {
        vec![Tool::Read]
    }

    fn permission_mode(&self) -> Option<PermissionMode> {
        Some(PermissionMode::Auto)
    }

    fn mcp_config(&self) -> Option<&str> {
        Some(r#"{"mcpServers":{}}"#)
    }

    fn model(&self) -> Option<&str> {
        Some(Model::HAIKU)
    }

    fn max_turns(&self) -> Option<u32> {
        Some(4)
    }
}

/// Asks the [`Reviewer`] role for a verdict.
struct RoleReview {
    seen: Arc<Mutex<Option<Verdict>>>,
}

impl WorkflowHandler for RoleReview {
    fn name(&self) -> &str {
        "role-review"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let verdict = ctx
                .agent("review", Reviewer.task("diff".into()).max_budget_usd(0.10))
                .await?;
            *self.seen.lock().expect("lock") = Some(verdict);
            Ok(())
        })
    }
}

/// Asks the [`Reviewer`] role for a verdict and overrides what the call may.
struct OverriddenReview;

impl WorkflowHandler for OverriddenReview {
    fn name(&self) -> &str {
        "overridden-review"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent(
                "review",
                Reviewer
                    .task("diff".into())
                    .model(Model::OPUS)
                    .max_turns(6)
                    .effort(Effort::High)
                    .working_dir("/repo")
                    .allow_failure()
                    .pod_label("team", "qa"),
            )
            .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_role_task_returns_the_role_output() {
    let seen = Arc::new(Mutex::new(None));
    let result = TestEngine::new()
        .with_handler(RoleReview { seen: seen.clone() })
        .with_mock_agent(|cfg| {
            assert_eq!(
                cfg.append_system_prompt.as_deref(),
                Some("You are a strict reviewer.")
            );
            assert_eq!(cfg.allowed_tools, vec!["Read"]);
            assert!(
                cfg.json_schema
                    .as_deref()
                    .is_some_and(|schema| schema.contains("approved")),
                "the schema of Verdict is sent to the provider"
            );
            assert!(matches!(cfg.permission_mode, PermissionMode::Auto));
            assert_eq!(cfg.mcp_config.as_deref(), Some(r#"{"mcpServers":{}}"#));
            assert_eq!(cfg.model, "haiku");
            assert_eq!(cfg.prompt, "Review: diff");
            Ok(AgentOutput::new(json!({"approved": true, "score": 9})))
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(
        *seen.lock().expect("lock"),
        Some(Verdict {
            approved: true,
            score: 9
        })
    );
}

#[tokio::test]
async fn call_site_overrides_reach_the_provider_and_keep_the_role() {
    let result = TestEngine::new()
        .with_handler(OverriddenReview)
        .with_mock_agent(|cfg| {
            assert_eq!(cfg.model, "opus");
            assert_eq!(cfg.max_turns, Some(6));
            assert_eq!(cfg.effort, Some(Effort::High));
            assert_eq!(cfg.working_dir.as_deref(), Some("/repo"));
            assert!(cfg.allow_failure);
            assert_eq!(cfg.pod_labels.get("team").map(String::as_str), Some("qa"));
            assert_eq!(
                cfg.append_system_prompt.as_deref(),
                Some("You are a strict reviewer.")
            );
            assert_eq!(cfg.allowed_tools, vec!["Read"]);
            assert!(
                cfg.json_schema
                    .as_deref()
                    .is_some_and(|schema| schema.contains("approved"))
            );
            Ok(AgentOutput::new(json!({"approved": true, "score": 5})))
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
}

#[tokio::test]
async fn an_answer_that_does_not_match_the_role_output_fails_the_run() {
    let seen = Arc::new(Mutex::new(None));
    let result = TestEngine::new()
        .with_handler(RoleReview { seen: seen.clone() })
        .with_mock_agent(|_cfg| Ok(AgentOutput::new(json!({"approved": "yes"}))))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Failed);
    let error = result.error().expect("the run records why it failed");
    assert!(error.starts_with("serialization error"), "got: {error}");
    assert!(seen.lock().expect("lock").is_none());
}

#[tokio::test]
async fn the_persisted_step_is_the_agent_config_of_the_role() {
    let seen = Arc::new(Mutex::new(None));
    let result = TestEngine::new()
        .with_handler(RoleReview { seen })
        .with_mock_agent(|_cfg| Ok(AgentOutput::new(json!({"approved": true, "score": 9}))))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    let by_hand = AgentStepConfig::new("Review: diff")
        .append_system_prompt("You are a strict reviewer.")
        .allow_tool(Tool::Read)
        .permission_mode(PermissionMode::Auto)
        .mcp_config(r#"{"mcpServers":{}}"#)
        .model(Model::HAIKU)
        .max_turns(4)
        .max_budget_usd(0.10)
        .output::<Verdict>()
        .into_config();
    assert_eq!(
        *result.step("review").input(),
        to_value(StepConfig::Agent(by_hand)).expect("serialize the config")
    );
}

/// Provider answering with a fixed verdict and counting invocations.
#[derive(Default)]
struct CountingProvider {
    calls: AtomicUsize,
}

impl AgentProvider for CountingProvider {
    fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(AgentOutput::new(json!({"approved": true, "score": 9})))
        })
    }
}

/// Reviews through the role, suspends on a human input and records the verdict
/// each time the handler runs.
struct ReviewThenAsk {
    verdicts: Arc<Mutex<Vec<Verdict>>>,
}

impl WorkflowHandler for ReviewThenAsk {
    fn name(&self) -> &str {
        "review-then-ask"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let verdict = ctx
                .agent("review", Reviewer.task("diff".into()).max_budget_usd(0.10))
                .await?;
            self.verdicts.lock().expect("lock").push(verdict);
            ctx.human_input::<Verdict>("clarify", HumanInputConfig::new("Approve?"))
                .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_replayed_role_step_is_not_called_again() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(CountingProvider::default());
        let verdicts = Arc::new(Mutex::new(Vec::new()));
        let dyn_store: Arc<dyn Store> = store.clone();
        let dyn_provider: Arc<dyn AgentProvider> = provider.clone();
        let mut engine = Engine::new(dyn_store, dyn_provider);
        engine
            .register(ReviewThenAsk {
                verdicts: verdicts.clone(),
            })
            .expect("register handler");

        let run = engine
            .enqueue_handler_with_options(
                "review-then-ask",
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
        let input_step = steps
            .iter()
            .find(|s| s.status.state == StepStatus::AwaitingApproval)
            .expect("human input step suspended the run");
        store
            .update_step(
                input_step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(json!({"approved": true, "score": 1})),
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

        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            1,
            "the role step is replayed"
        );
        let verdicts = verdicts.lock().expect("lock");
        assert_eq!(verdicts.len(), 2);
        assert_eq!(verdicts[0], verdicts[1]);
        assert_eq!(
            verdicts[0],
            Verdict {
                approved: true,
                score: 9
            }
        );
    })
    .await
    .expect("test timed out");
}

/// Reviews through the role, with or without a model override.
struct FixtureReview {
    override_model: bool,
}

impl WorkflowHandler for FixtureReview {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let task = Reviewer.task("diff".into());
            let task = if self.override_model {
                task.model(Model::OPUS)
            } else {
                task
            };
            ctx.agent("review", task).await?;
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

/// Records the verdict under the hash of the role's default config.
fn fixtures_dir() -> (String, FixtureGuard) {
    let dir = temp_dir()
        .join(format!(
            "ironflow-agent-role-{}-{}",
            process_id(),
            Uuid::now_v7()
        ))
        .display()
        .to_string();
    let guard = FixtureGuard(dir.clone());

    let config = Reviewer.task("diff".into()).into_config();
    let output = AgentOutput::new(json!({"approved": true, "score": 9}));

    create_dir_all(&dir).expect("create fixtures dir");
    let fixture = json!({ "config": config, "output": output });
    write(
        format!("{dir}/{}.json", hash_config(&config)),
        to_string(&fixture).expect("serialize fixture"),
    )
    .expect("write fixture");

    (dir, guard)
}

/// An engine replaying `dir`, with a [`FixtureReview`] registered.
fn replay_engine(dir: &str, override_model: bool) -> (Engine, Arc<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let dyn_store: Arc<dyn Store> = store.clone();
    let provider: Arc<dyn AgentProvider> =
        Arc::new(RecordReplayProvider::replay(MissingAgentProvider, dir));
    let mut engine = Engine::new(dyn_store, provider);
    engine
        .register(FixtureReview { override_model })
        .expect("register handler");
    (engine, store)
}

#[tokio::test]
async fn a_call_site_override_misses_the_fixture_recorded_for_the_role_default() {
    let (dir, _guard) = fixtures_dir();

    timeout(TEST_TIMEOUT, async {
        let (engine, store) = replay_engine(&dir, false);
        let result = engine
            .run_handler(WORKFLOW, TriggerKind::Api, json!({}))
            .await
            .expect("run completes from the fixture");
        assert_eq!(result.run.status.state, RunStatus::Completed);
        let steps = store.list_steps(result.run.id).await.expect("list steps");
        let step = steps
            .iter()
            .find(|s| s.name == "review")
            .expect("agent step stored");
        assert_eq!(
            step.output,
            Some(json!({"approved": true, "score": 9})),
            "the fixture's answer: replayed, not asked to the real CLI"
        );
    })
    .await
    .expect("test timed out");

    timeout(TEST_TIMEOUT, async {
        let (engine, _store) = replay_engine(&dir, true);
        let result = engine
            .run_handler(WORKFLOW, TriggerKind::Api, json!({}))
            .await
            .expect("the engine finalizes the failed run");
        assert_eq!(
            result.run.status.state,
            RunStatus::Failed,
            "the overridden config hashes to no fixture"
        );
    })
    .await
    .expect("test timed out");
}
