//! Integration tests for agent steps with a typed answer and typed tools.
//!
//! `ctx.agent` with `.output::<T>()` returns the `T` itself. The agent provider
//! is the harness mock: it plays the model, the engine code path is real.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture, Tool};
use ironflow_engine::config::{AgentStepConfig, HumanInputConfig, ShellConfig};
use ironflow_engine::context::{AgentReply, WorkflowContext};
use ironflow_engine::engine::{Engine, EnqueueOptions};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::testing::{MockShellOutput, TestEngine};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Prompt and environment id of each resumed agent call, in order.
type ResumeLog = Vec<(String, Option<String>)>;

/// What the reviewer agent answers.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
struct Verdict {
    approved: bool,
    score: u8,
}

/// Asks for a [`Verdict`] and ships when it is approved.
struct Reviewer {
    seen: Arc<Mutex<Option<Verdict>>>,
}

impl WorkflowHandler for Reviewer {
    fn name(&self) -> &str {
        "reviewer"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let verdict = ctx
                .agent(
                    "review",
                    AgentStepConfig::new("Review the release")
                        .max_turns(2)
                        .max_budget_usd(0.10)
                        .output::<Verdict>(),
                )
                .await?;
            if verdict.approved {
                ctx.shell("ship", ShellConfig::new("./ship")).await?;
            }
            *self.seen.lock().expect("lock") = Some(verdict);
            Ok(())
        })
    }
}

/// Explores with tools; its answer stays a plain step output.
struct Explorer;

impl WorkflowHandler for Explorer {
    fn name(&self) -> &str {
        "explorer"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let found = ctx
                .agent(
                    "explore",
                    AgentStepConfig::new("List the crates")
                        .max_budget_usd(0.10)
                        .allow_tool(Tool::Bash)
                        .allow_tool(Tool::Custom("mcp__repo__tree".to_string())),
                )
                .await?;
            ctx.shell(
                "echo",
                ShellConfig::new("echo \"$FOUND\"").env("FOUND", &found.output.to_string()),
            )
            .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_typed_agent_step_returns_its_answer() {
    let seen = Arc::new(Mutex::new(None));
    let result = TestEngine::new()
        .with_handler(Reviewer { seen: seen.clone() })
        .with_mock_agent(|cfg| {
            assert!(
                cfg.json_schema
                    .as_deref()
                    .is_some_and(|schema| schema.contains("approved")),
                "the schema of Verdict is sent to the provider"
            );
            Ok(AgentOutput::new(json!({"approved": true, "score": 9})))
        })
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("shipped")))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(result.step_names(), vec!["review", "ship"]);
    assert_eq!(
        *seen.lock().expect("lock"),
        Some(Verdict {
            approved: true,
            score: 9
        })
    );
}

#[tokio::test]
async fn a_rejected_typed_answer_skips_the_next_step() {
    let seen = Arc::new(Mutex::new(None));
    let result = TestEngine::new()
        .with_handler(Reviewer { seen: seen.clone() })
        .with_mock_agent(|_cfg| Ok(AgentOutput::new(json!({"approved": false, "score": 2}))))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(result.step_names(), vec!["review"]);
    assert_eq!(
        seen.lock().expect("lock").as_ref().map(|v| v.score),
        Some(2)
    );
}

#[tokio::test]
async fn an_answer_that_does_not_match_the_type_fails_the_run() {
    let seen = Arc::new(Mutex::new(None));
    let result = TestEngine::new()
        .with_handler(Reviewer { seen: seen.clone() })
        .with_mock_agent(|_cfg| Ok(AgentOutput::new(json!({"approved": "yes"}))))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Failed);
    let error = result.error().expect("the run records why it failed");
    assert!(error.starts_with("serialization error"), "got: {error}");
    assert!(error.contains("expected a boolean"), "got: {error}");
    assert!(seen.lock().expect("lock").is_none());
}

#[tokio::test]
async fn typed_tools_reach_the_provider_under_their_names() {
    let result = TestEngine::new()
        .with_handler(Explorer)
        .with_mock_agent(|cfg| {
            assert_eq!(cfg.allowed_tools, vec!["Bash", "mcp__repo__tree"]);
            Ok(AgentOutput::new(json!("ironflow-core")))
        })
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ironflow-core")))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(result.step_names(), vec!["explore", "echo"]);
}

/// Clones in a typed step and keeps the metadata next to the answer.
struct Cloner {
    seen: Arc<Mutex<Option<AgentReply<Verdict>>>>,
}

impl WorkflowHandler for Cloner {
    fn name(&self) -> &str {
        "cloner"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let reply = ctx
                .agent_with_meta(
                    "clone",
                    AgentStepConfig::new("Clone the repository")
                        .max_budget_usd(0.10)
                        .output::<Verdict>(),
                )
                .await?;
            *self.seen.lock().expect("lock") = Some(reply);
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_typed_agent_step_exposes_its_environment_id() {
    let seen = Arc::new(Mutex::new(None));
    let account = Uuid::now_v7();
    let result = TestEngine::new()
        .with_handler(Cloner { seen: seen.clone() })
        .with_mock_agent(move |_cfg| {
            let mut output = AgentOutput::new(json!({"approved": true, "score": 7}));
            output.environment_id = Some("ironflow-env-0a1b2c".to_string());
            output.account_id = Some(account.to_string());
            Ok(output)
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(
        *seen.lock().expect("lock"),
        Some(AgentReply {
            answer: Verdict {
                approved: true,
                score: 7
            },
            environment_id: Some("ironflow-env-0a1b2c".to_string()),
            account_id: Some(account),
        })
    );
}

#[tokio::test]
async fn a_typed_agent_step_without_environment_has_no_ids() {
    let seen = Arc::new(Mutex::new(None));
    let result = TestEngine::new()
        .with_handler(Cloner { seen: seen.clone() })
        .with_mock_agent(|_cfg| Ok(AgentOutput::new(json!({"approved": false, "score": 1}))))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    let reply = seen.lock().expect("lock").clone().expect("a reply");
    assert_eq!(reply.answer.score, 1);
    assert_eq!(reply.environment_id, None);
    assert_eq!(reply.account_id, None);
}

#[tokio::test]
async fn a_mismatched_answer_fails_agent_with_meta_without_a_reply() {
    let seen = Arc::new(Mutex::new(None));
    let result = TestEngine::new()
        .with_handler(Cloner { seen: seen.clone() })
        .with_mock_agent(|_cfg| {
            let mut output = AgentOutput::new(json!({"approved": "yes"}));
            output.environment_id = Some("ironflow-env-0a1b2c".to_string());
            Ok(output)
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Failed);
    let error = result.error().expect("the run records why it failed");
    assert!(error.starts_with("serialization error"), "got: {error}");
    assert!(seen.lock().expect("lock").is_none());
}

/// Clones in a typed step, then resumes the environment it handed out.
struct CloneThenFix;

impl WorkflowHandler for CloneThenFix {
    fn name(&self) -> &str {
        "clone-then-fix"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let reply = ctx
                .agent_with_meta(
                    "clone",
                    AgentStepConfig::new("Clone the repository")
                        .max_budget_usd(0.10)
                        .output::<Verdict>(),
                )
                .await?;
            let environment = reply
                .environment_id
                .expect("the clone step hands out an environment id");
            ctx.agent(
                "fix",
                AgentStepConfig::new("Fix the test")
                    .max_budget_usd(0.10)
                    .resume_environment(&environment),
            )
            .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn the_environment_id_of_a_typed_step_reaches_the_next_step() {
    let resumed: Arc<Mutex<ResumeLog>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = resumed.clone();
    let result = TestEngine::new()
        .with_handler(CloneThenFix)
        .with_mock_agent(move |cfg| {
            recorder
                .lock()
                .expect("lock")
                .push((cfg.prompt.clone(), cfg.resume_environment_id.clone()));
            let mut output = AgentOutput::new(json!({"approved": true, "score": 3}));
            output.environment_id = Some("ironflow-env-0a1b2c".to_string());
            Ok(output)
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(result.step_names(), vec!["clone", "fix"]);
    assert_eq!(
        *resumed.lock().expect("lock"),
        vec![
            ("Clone the repository".to_string(), None),
            (
                "Fix the test".to_string(),
                Some("ironflow-env-0a1b2c".to_string())
            ),
        ]
    );
}

/// Provider handing out a fixed environment id and counting invocations.
#[derive(Default)]
struct CountingProvider {
    calls: AtomicUsize,
}

impl AgentProvider for CountingProvider {
    fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut output = AgentOutput::new(json!({"approved": true, "score": 9}));
            output.environment_id = Some("ironflow-env-1".to_string());
            Ok(output)
        })
    }
}

/// Clones in a typed step, suspends on a human input and records the reply of
/// the clone step each time the handler runs.
struct CloneThenAsk {
    replies: Arc<Mutex<Vec<AgentReply<Verdict>>>>,
}

impl WorkflowHandler for CloneThenAsk {
    fn name(&self) -> &str {
        "clone-then-ask"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let reply = ctx
                .agent_with_meta(
                    "clone",
                    AgentStepConfig::new("Clone the repository")
                        .max_budget_usd(0.10)
                        .output::<Verdict>(),
                )
                .await?;
            self.replies.lock().expect("lock").push(reply);
            ctx.human_input::<Verdict>("clarify", HumanInputConfig::new("Approve?"))
                .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_replayed_typed_step_returns_the_same_environment_id() {
    timeout(Duration::from_secs(10), async {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(CountingProvider::default());
        let replies = Arc::new(Mutex::new(Vec::new()));
        let dyn_store: Arc<dyn Store> = store.clone();
        let dyn_provider: Arc<dyn AgentProvider> = provider.clone();
        let mut engine = Engine::new(dyn_store, dyn_provider);
        engine
            .register(CloneThenAsk {
                replies: replies.clone(),
            })
            .expect("register handler");

        let run = engine
            .enqueue_handler_with_options(
                "clone-then-ask",
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
            "clone is replayed"
        );
        let replies = replies.lock().expect("lock");
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0].environment_id.as_deref(), Some("ironflow-env-1"));
        assert_eq!(replies[0], replies[1]);
    })
    .await
    .expect("test timed out");
}
