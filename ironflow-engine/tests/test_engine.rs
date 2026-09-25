//! Integration tests for the in-memory test harness.
//!
//! Every test drives [`TestEngine`] through its public API only, the way a
//! workflow author would. No process is spawned, no socket is opened and no
//! agent backend is reached: an assertion that passes here proves the mocks
//! really short-circuited the step.
//!
//! Test names are prefixed `test_engine_` so `cargo test -p ironflow-engine --
//! test_engine` selects the whole suite.

use std::sync::Arc;
use std::time::Duration;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_str, json};
use tempfile::tempdir;

use ironflow_core::error::OperationError;
use ironflow_core::provider::{AgentOutput, AgentProvider};
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{
    AgentStepConfig, ApprovalConfig, HttpConfig, ShellConfig, StepConfig,
};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::executor::SubWorkflowOutput;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::testing::{
    ApprovalOutcome, MockAgentProvider, MockHttpResponse, MockShellOutput, TestEngine,
};
use ironflow_store::models::{RunStatus, StepKind, StepStatus};
use ironflow_store::store::RunStore;

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

/// Three sequential shell steps.
struct Deploy;

impl WorkflowHandler for Deploy {
    fn name(&self) -> &str {
        "deploy"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("cargo build")).await?;
            ctx.shell("test", ShellConfig::new("cargo test")).await?;
            ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?;
            Ok(())
        })
    }
}

/// A shell step whose failure must not stop the run.
struct TolerantDeploy;

impl WorkflowHandler for TolerantDeploy {
    fn name(&self) -> &str {
        "tolerant-deploy"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("lint", ShellConfig::new("cargo clippy").allow_failure())
                .await?;
            ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?;
            Ok(())
        })
    }
}

/// A shell step followed by an HTTP call.
struct Fetch;

impl WorkflowHandler for Fetch {
    fn name(&self) -> &str {
        "fetch"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("prepare", ShellConfig::new("mkdir -p out"))
                .await?;
            ctx.http(
                "call",
                HttpConfig::post("https://example.test/things").json(json!({"name": "thing"})),
            )
            .await?;
            Ok(())
        })
    }
}

/// A single agent step.
struct Review;

impl WorkflowHandler for Review {
    fn name(&self) -> &str {
        "review"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("review", AgentStepConfig::new("Review the release"))
                .await?;
            Ok(())
        })
    }
}

/// Shell, approval gate, shell.
struct GatedDeploy;

impl WorkflowHandler for GatedDeploy {
    fn name(&self) -> &str {
        "gated-deploy"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("cargo build")).await?;
            ctx.approval("gate", ApprovalConfig::new("Ship to production?"))
                .await?;
            ctx.shell("ship", ShellConfig::new("./ship.sh")).await?;
            Ok(())
        })
    }
}

/// Two steps in one parallel wave.
struct ParallelChecks;

impl WorkflowHandler for ParallelChecks {
    fn name(&self) -> &str {
        "parallel-checks"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.parallel(
                vec![
                    ("lint", StepConfig::Shell(ShellConfig::new("cargo clippy"))),
                    ("unit", StepConfig::Shell(ShellConfig::new("cargo test"))),
                ],
                true,
            )
            .await?;
            Ok(())
        })
    }
}

/// A failing step with an `on_error` handler registered before it.
struct CleanUpOnFailure;

impl WorkflowHandler for CleanUpOnFailure {
    fn name(&self) -> &str {
        "cleanup-on-failure"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.on_error("cleanup", ShellConfig::new("rm -rf /tmp/build"));
            ctx.shell("build", ShellConfig::new("cargo build")).await?;
            Ok(())
        })
    }
}

/// The child of [`Parent`].
struct Child;

/// Input of [`Child`].
#[derive(Serialize, Deserialize)]
struct ChildInput {
    from: String,
}

impl TypedWorkflow for Child {
    type Input = ChildInput;
}

impl WorkflowHandler for Child {
    fn name(&self) -> &str {
        "child"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("child-step", ShellConfig::new("./run-child.sh"))
                .await?;
            Ok(())
        })
    }
}

/// Invokes [`Child`] as a sub-workflow.
struct Parent;

impl WorkflowHandler for Parent {
    fn name(&self) -> &str {
        "parent"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(
                &Child,
                ChildInput {
                    from: "parent".to_string(),
                },
            )
            .await?;
            Ok(())
        })
    }
}

/// Typed payload of [`Greet`].
#[derive(Deserialize)]
struct GreetInput {
    name: String,
}

/// Reads its typed payload and echoes it through a shell step.
struct Greet;

impl WorkflowHandler for Greet {
    fn name(&self) -> &str {
        "greet"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: GreetInput = ctx.input().await?;
            ctx.shell("greet", ShellConfig::new(&format!("echo {}", input.name)))
                .await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Shell mocks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_engine_completes_a_run_with_mocked_shell_steps() {
    let result = TestEngine::new()
        .with_handler(Deploy)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("done")))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert!(result.is_completed());
    assert_eq!(result.steps().len(), 3);
    assert_eq!(result.step_names(), vec!["build", "test", "deploy"]);
    assert_eq!(result.step("deploy").output()["stdout"], "done");
    assert_eq!(result.step("deploy").kind(), &StepKind::Shell);
    assert!(result.steps().iter().all(|step| step.is_completed()));
    assert!(result.error().is_none());
}

#[tokio::test]
async fn test_engine_shell_mock_sees_the_configured_command() {
    let result = TestEngine::new()
        .with_handler(Deploy)
        .with_mock_shell(|cfg| match cfg.command.as_str() {
            "cargo build" => Ok(MockShellOutput::ok("compiled")),
            "cargo test" => Ok(MockShellOutput::ok("42 passed")),
            other => Ok(MockShellOutput::ok(other)),
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.step("build").output()["stdout"], "compiled");
    assert_eq!(result.step("test").output()["stdout"], "42 passed");
    assert_eq!(result.step("deploy").output()["stdout"], "./deploy.sh");
}

#[tokio::test]
async fn test_engine_shell_mock_failure_fails_the_run() {
    let result = TestEngine::new()
        .with_handler(Deploy)
        .with_mock_shell(|cfg| {
            if cfg.command == "cargo build" {
                Ok(MockShellOutput::failed(1, "boom"))
            } else {
                Ok(MockShellOutput::ok("done"))
            }
        })
        .run(json!({}))
        .await
        .expect("a failing handler is still a result, not an Err");

    assert_eq!(result.status(), RunStatus::Failed);
    let error = result.error().expect("the run carries an error");
    assert!(error.contains("boom"), "unexpected error: {error}");
    assert_eq!(result.step("build").status(), StepStatus::Failed);
    // The steps after the failing one were never created.
    assert_eq!(result.step_names(), vec!["build"]);
    assert!(result.try_step("deploy").is_none());
}

#[tokio::test]
async fn test_engine_shell_mock_failure_with_allow_failure_warns() {
    let result = TestEngine::new()
        .with_handler(TolerantDeploy)
        .with_mock_shell(|cfg| {
            if cfg.command == "cargo clippy" {
                Ok(MockShellOutput::failed(1, "3 warnings"))
            } else {
                Ok(MockShellOutput::ok("shipped"))
            }
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Warning);
    assert_eq!(result.step("lint").status(), StepStatus::Failed);
    assert_eq!(result.step("deploy").output()["stdout"], "shipped");
}

// ---------------------------------------------------------------------------
// HTTP mocks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_engine_http_mock_returns_status_and_body() {
    let result = TestEngine::new()
        .with_handler(Fetch)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ready")))
        .with_mock_http(|cfg| {
            assert_eq!(cfg.method, "POST");
            assert_eq!(cfg.url, "https://example.test/things");
            Ok(MockHttpResponse::json(201, &json!({"id": 7})).header("location", "/things/7"))
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    let call = result.step("call");
    assert_eq!(call.output()["status"], 201);
    assert_eq!(call.output()["headers"]["location"], "/things/7");

    let raw_body = call.output()["body"].as_str().expect("a string body");
    let body: Value = from_str(raw_body).expect("the body is JSON");
    assert_eq!(body["id"], 7);
}

#[tokio::test]
async fn test_engine_http_mock_transport_error_fails_the_run() {
    let result = TestEngine::new()
        .with_handler(Fetch)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ready")))
        .with_mock_http(|_cfg| {
            Err(OperationError::Http {
                status: None,
                message: "connection refused".to_string(),
            })
        })
        .run(json!({}))
        .await
        .expect("a failing handler is still a result, not an Err");

    assert_eq!(result.status(), RunStatus::Failed);
    let error = result.error().expect("the run carries an error");
    assert!(error.contains("connection refused"), "got: {error}");
    assert_eq!(result.step("call").status(), StepStatus::Failed);
}

// ---------------------------------------------------------------------------
// Agent mocks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_engine_agent_mock_output_is_persisted() {
    let result = TestEngine::new()
        .with_handler(Review)
        .with_mock_agent(|cfg| {
            assert!(cfg.prompt.contains("Review the release"));
            let mut output = AgentOutput::new(json!({"version": "1.2.3"}));
            output.cost_usd = Some(0.25);
            output.input_tokens = Some(120);
            output.output_tokens = Some(45);
            Ok(output)
        })
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    let review = result.step("review");
    assert_eq!(review.output()["version"], "1.2.3");
    assert_eq!(review.kind(), &StepKind::Agent);
    assert_eq!(review.cost_usd(), Decimal::new(25, 2));
    assert_eq!(review.raw().input_tokens, Some(120));
    assert_eq!(review.raw().output_tokens, Some(45));
    assert_eq!(result.cost_usd(), Decimal::new(25, 2));
}

#[tokio::test]
async fn test_engine_without_agent_provider_reports_a_helpful_error() {
    let result = TestEngine::new()
        .with_handler(Review)
        .run(json!({}))
        .await
        .expect("a failing handler is still a result, not an Err");

    assert_eq!(result.status(), RunStatus::Failed);
    let error = result.error().expect("the run carries an error");
    assert!(error.contains("with_mock_agent"), "got: {error}");
    assert!(error.contains("with_recorded_agent"));
}

#[tokio::test]
async fn test_engine_recorded_agent_replays_a_fixture() {
    let dir = tempdir().expect("temp dir");
    let fixtures = dir.path().to_str().expect("utf-8 path").to_string();

    // First pass: record what the mock answers.
    let recorder: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::record(
        MockAgentProvider::new(|_cfg| Ok(AgentOutput::new(json!({"verdict": "ship it"})))),
        &fixtures,
    ));
    let recorded = TestEngine::new()
        .with_handler(Review)
        .with_agent_provider(recorder)
        .run(json!({}))
        .await
        .expect("the recording run succeeded");
    assert_eq!(recorded.step("review").output()["verdict"], "ship it");

    // Second pass: replay it, with no backend behind the fixture at all.
    let replayed = TestEngine::new()
        .with_handler(Review)
        .with_recorded_agent(&fixtures)
        .run(json!({}))
        .await
        .expect("the replay run succeeded");

    assert_eq!(replayed.status(), RunStatus::Completed);
    assert_eq!(replayed.step("review").output()["verdict"], "ship it");
}

// ---------------------------------------------------------------------------
// Approval gates
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_engine_mock_approval_grants_the_gate() {
    let result = TestEngine::new()
        .with_handler(GatedDeploy)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ok")))
        .with_mock_approval(ApprovalOutcome::Approved)
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(result.step_names(), vec!["build", "gate", "ship"]);
    assert_eq!(result.step("gate").status(), StepStatus::Completed);
    assert_eq!(result.step("gate").kind(), &StepKind::Approval);
}

#[tokio::test]
async fn test_engine_mock_approval_rejects_the_gate() {
    let result = TestEngine::new()
        .with_handler(GatedDeploy)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ok")))
        .with_mock_approval(ApprovalOutcome::reject("nope"))
        .run(json!({}))
        .await
        .expect("a rejected gate is still a result, not an Err");

    assert_eq!(result.status(), RunStatus::Failed);
    let error = result.error().expect("the run carries an error");
    assert!(error.contains("nope"), "unexpected error: {error}");
    assert_eq!(result.step("gate").status(), StepStatus::Rejected);
    assert_eq!(result.step("gate").error(), Some("nope"));
    // The step behind the gate was never created.
    assert!(result.try_step("ship").is_none());
}

#[tokio::test]
async fn test_engine_without_approval_mock_suspends_and_resumes() {
    let mut harness = TestEngine::new()
        .with_handler(GatedDeploy)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ok")));

    let suspended = harness.run(json!({})).await.expect("the first pass ran");
    assert_eq!(suspended.status(), RunStatus::AwaitingApproval);
    assert_eq!(
        suspended.step("gate").status(),
        StepStatus::AwaitingApproval
    );
    assert!(suspended.try_step("ship").is_none());

    let resumed = harness
        .resume(suspended.run_id())
        .await
        .expect("the resume pass ran");

    assert_eq!(resumed.status(), RunStatus::Completed);
    assert_eq!(resumed.run_id(), suspended.run_id());
    assert_eq!(resumed.step_names(), vec!["build", "gate", "ship"]);
    assert_eq!(resumed.step("gate").status(), StepStatus::Completed);
}

// ---------------------------------------------------------------------------
// Parallel waves, error handlers, sub-workflows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_engine_mocks_steps_inside_a_parallel_wave() {
    let result = TestEngine::new()
        .with_handler(ParallelChecks)
        .with_mock_shell(|cfg| Ok(MockShellOutput::ok(&cfg.command)))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(result.steps().len(), 2);
    assert!(result.steps().iter().all(|step| step.is_completed()));
    assert_eq!(result.step("lint").output()["stdout"], "cargo clippy");
    assert_eq!(result.step("unit").output()["stdout"], "cargo test");
    // Both steps belong to the same wave.
    let lint_position = result.step("lint").raw().position;
    assert_eq!(lint_position, result.step("unit").raw().position);
}

#[tokio::test]
async fn test_engine_mocks_an_on_error_handler_step() {
    let result = TestEngine::new()
        .with_handler(CleanUpOnFailure)
        .with_mock_shell(|cfg| {
            if cfg.command == "cargo build" {
                Ok(MockShellOutput::failed(1, "compile error"))
            } else {
                Ok(MockShellOutput::ok("cleaned"))
            }
        })
        .run(json!({}))
        .await
        .expect("a failing handler is still a result, not an Err");

    assert_eq!(result.status(), RunStatus::Failed);
    let cleanup = result.step("cleanup");
    assert!(cleanup.is_error_handler());
    assert_eq!(cleanup.status(), StepStatus::Completed);
    assert_eq!(cleanup.output()["stdout"], "cleaned");
    assert!(!result.step("build").is_error_handler());
}

#[tokio::test]
async fn test_engine_mocks_steps_of_a_sub_workflow() {
    let mut harness = TestEngine::new()
        .with_handler(Parent)
        .with_handler(Child)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("child ran")));
    let store = harness.store();

    let result = harness
        .run_workflow("parent", json!({}))
        .await
        .expect("the harness ran the parent");

    assert_eq!(result.status(), RunStatus::Completed);
    let child_step = result.step("child");
    assert_eq!(child_step.kind(), &StepKind::Workflow);

    let child: SubWorkflowOutput = child_step
        .step_output()
        .json()
        .expect("the child run is recorded");
    assert_eq!(child.workflow_name(), "child");
    assert_eq!(child.status(), RunStatus::Completed);
    let child_steps = store.list_steps(child.run_id()).await.expect("list steps");

    assert_eq!(child_steps.len(), 1);
    assert_eq!(child_steps[0].name, "child-step");
    assert_eq!(child_steps[0].status.state, StepStatus::Completed);
    let child_output = child_steps[0].output.as_ref().expect("has output");
    assert_eq!(child_output["stdout"], "child ran");
}

// ---------------------------------------------------------------------------
// Wiring errors and result accessors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_engine_run_without_handler_returns_invalid_workflow() {
    let err = TestEngine::new()
        .run(json!({}))
        .await
        .expect_err("no handler was registered");

    assert!(matches!(err, EngineError::InvalidWorkflow(_)));
    assert!(err.to_string().contains("with_handler"));
}

#[tokio::test]
async fn test_engine_try_step_returns_none_for_an_unknown_name() {
    let result = TestEngine::new()
        .with_handler(Deploy)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("done")))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    assert!(result.try_step("nope").is_none());
}

#[tokio::test]
#[should_panic(expected = "no step named \"nope\"")]
async fn test_engine_step_panics_for_an_unknown_name() {
    let result = TestEngine::new()
        .with_handler(Deploy)
        .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("done")))
        .run(json!({}))
        .await
        .expect("the harness ran the handler");

    result.step("nope");
}

#[tokio::test]
async fn test_engine_exposes_duration_cost_and_payload() {
    let result = TestEngine::new()
        .with_handler(Greet)
        .with_mock_shell(|cfg| Ok(MockShellOutput::ok(&cfg.command)))
        .run(json!({"name": "ada"}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    // The handler read its typed payload from the run.
    assert_eq!(result.step("greet").output()["stdout"], "echo ada");
    assert_eq!(result.output()["stdout"], "echo ada");
    // Mocked steps cost nothing and the run duration is recorded.
    assert_eq!(result.cost_usd(), Decimal::ZERO);
    assert_eq!(
        result.duration(),
        Duration::from_millis(result.run().duration_ms)
    );
    assert_eq!(result.run().payload["name"], "ada");
}
