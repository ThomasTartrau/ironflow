//! End-to-end tests for tool profiles chosen step by step.
//!
//! The workflow runs on the real engine with a real `OpenAiProvider`, pointed
//! at a local server that plays the OpenAI chat completions API and records
//! every request, so the tests assert what each step actually sent.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router, serve};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::time::timeout;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::http::OpenAiProvider;
use ironflow_core::providers::http::tools::{Tool, ToolError, ToolOutput, ToolRegistry};
use ironflow_engine::config::{AgentStepConfig, ToolProfile};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::testing::TestEngine;
use ironflow_store::models::{RunStatus, StepStatus};

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

const SUGGESTION: ToolProfile = ToolProfile::new("suggestion");
const BUG: ToolProfile = ToolProfile::new("bug");
/// Declared by the workflow but not registered on the worker's provider.
const INCIDENT: ToolProfile = ToolProfile::new("incident");

/// Request bodies received by the fake LLM, in arrival order.
type Received = Arc<Mutex<Vec<Value>>>;

async fn chat_completions(
    State(received): State<Received>,
    Json(body): Json<Value>,
) -> Json<Value> {
    received.lock().expect("lock").push(body);
    Json(json!({
        "model": "gpt-test",
        "choices": [{"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2}
    }))
}

/// Start the fake LLM and return its base URL.
async fn fake_llm() -> (String, Received) {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/chat/completions", post(chat_completions))
        .with_state(received.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    spawn(async move {
        serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), received)
}

/// A tool that only has a name: these tests check what is exposed.
struct NamedTool(&'static str);

impl Tool for NamedTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "A tool"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(
        &self,
        _input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async { Ok(ToolOutput::success("ran")) })
    }
}

fn registry(names: &[&'static str]) -> ToolRegistry {
    names.iter().fold(ToolRegistry::new(), |reg, name| {
        reg.register(NamedTool(name))
    })
}

/// An OpenAI provider with a "suggestion" and a "bug" profile.
fn profiled_provider(base_url: &str) -> Arc<dyn AgentProvider> {
    Arc::new(
        OpenAiProvider::with_credentials("test-key".to_string(), base_url.to_string())
            .with_tool_profile(SUGGESTION, registry(&["code_search", "gitlab_mr"]))
            .with_tool_profile(
                BUG,
                registry(&["code_search", "gitlab_mr", "grafana_query", "sentry_issue"]),
            ),
    )
}

fn step(prompt: &str) -> AgentStepConfig {
    AgentStepConfig::new(prompt)
        .model("gpt-test")
        .max_budget_usd(0.10)
}

/// Names of the tools in a request body, or `None` without a `tools` key.
fn tool_names(request: &Value) -> Option<Vec<&str>> {
    request.get("tools").map(|tools| {
        tools
            .as_array()
            .expect("tools is an array")
            .iter()
            .map(|t| t["function"]["name"].as_str().expect("tool name"))
            .collect()
    })
}

/// Suggests with one profile, investigates with another, summarizes with none.
struct Triage;

impl WorkflowHandler for Triage {
    fn name(&self) -> &str {
        "triage"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("suggest", step("Suggest a fix").tool_profile(SUGGESTION))
                .await?;
            ctx.agent("investigate", step("Find the root cause").tool_profile(BUG))
                .await?;
            ctx.agent("summarize", step("Summarize")).await?;
            Ok(())
        })
    }
}

/// Asks for a profile the provider does not have.
struct Unregistered;

impl WorkflowHandler for Unregistered {
    fn name(&self) -> &str {
        "unregistered"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent(
                "investigate",
                step("Find the root cause").tool_profile(INCIDENT),
            )
            .await?;
            ctx.agent("summarize", step("Summarize")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn tool_profile_each_step_sees_only_its_tools() {
    timeout(TEST_TIMEOUT, async {
        let (url, received) = fake_llm().await;
        let result = TestEngine::new()
            .with_handler(Triage)
            .with_agent_provider(profiled_provider(&url))
            .run(json!({}))
            .await
            .expect("the harness ran the handler");

        assert_eq!(result.status(), RunStatus::Completed);
        assert_eq!(
            result.step_names(),
            vec!["suggest", "investigate", "summarize"]
        );

        let requests = received.lock().expect("lock");
        assert_eq!(requests.len(), 3);
        assert_eq!(
            tool_names(&requests[0]),
            Some(vec!["code_search", "gitlab_mr"])
        );
        assert_eq!(
            tool_names(&requests[1]),
            Some(vec![
                "code_search",
                "gitlab_mr",
                "grafana_query",
                "sentry_issue"
            ])
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn tool_profile_absent_sends_no_tools() {
    timeout(TEST_TIMEOUT, async {
        let (url, received) = fake_llm().await;
        let result = TestEngine::new()
            .with_handler(Triage)
            .with_agent_provider(profiled_provider(&url))
            .run(json!({}))
            .await
            .expect("the harness ran the handler");

        assert_eq!(result.status(), RunStatus::Completed);
        let requests = received.lock().expect("lock");
        assert_eq!(requests[2]["messages"][0]["content"], "Summarize");
        assert_eq!(tool_names(&requests[2]), None);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn tool_profile_is_recorded_in_the_step_input() {
    timeout(TEST_TIMEOUT, async {
        let (url, _received) = fake_llm().await;
        let result = TestEngine::new()
            .with_handler(Triage)
            .with_agent_provider(profiled_provider(&url))
            .run(json!({}))
            .await
            .expect("the harness ran the handler");

        assert_eq!(result.step("suggest").input()["tool_profile"], "suggestion");
        assert_eq!(result.step("investigate").input()["tool_profile"], "bug");
        assert!(
            result
                .step("summarize")
                .input()
                .get("tool_profile")
                .is_none()
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn tool_profile_unknown_fails_the_step_and_the_run() {
    timeout(TEST_TIMEOUT, async {
        let (url, received) = fake_llm().await;
        let result = TestEngine::new()
            .with_handler(Unregistered)
            .with_agent_provider(profiled_provider(&url))
            .run(json!({}))
            .await
            .expect("the harness ran the handler");

        assert_eq!(result.status(), RunStatus::Failed);
        assert_eq!(result.step_names(), vec!["investigate"]);
        let step = result.step("investigate");
        assert_eq!(step.status(), StepStatus::Failed);
        assert_eq!(
            step.error(),
            Some(
                "operation failed: agent error: unknown tool profile 'incident' \
                 (registered profiles: bug, suggestion)"
            )
        );
        assert!(
            received.lock().expect("lock").is_empty(),
            "no request may reach the model with an unknown profile"
        );
    })
    .await
    .expect("test timed out");
}
