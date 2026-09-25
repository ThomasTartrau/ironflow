//! Integration tests for agent steps with a typed answer and typed tools.
//!
//! `ctx.agent` with `.output::<T>()` returns the `T` itself. The agent provider
//! is the harness mock: it plays the model, the engine code path is real.

use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use ironflow_core::provider::{AgentOutput, Tool};
use ironflow_engine::config::{AgentStepConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::testing::{MockShellOutput, TestEngine};
use ironflow_store::models::RunStatus;

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
