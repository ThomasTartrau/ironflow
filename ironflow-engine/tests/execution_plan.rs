//! Integration tests for execution plans: plan mode records without executing.

use std::fs::remove_file;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::delay::DelayConfig;
use ironflow_engine::config::{ApprovalConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::plan::{ConditionResult, PlanOptions};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunFilter, StepKind, TriggerKind};
use ironflow_store::store::RunStore;

/// Wall-clock budget for a single test; a plan must never block on a process.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn test_store() -> Arc<InMemoryStore> {
    Arc::new(InMemoryStore::new())
}

fn engine_with(store: Arc<InMemoryStore>) -> Engine {
    let inner = ClaudeCodeProvider::new();
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
        inner,
        "/tmp/ironflow-fixtures",
    ));
    Engine::new(store, provider)
}

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

struct LinearWorkflow;

impl WorkflowHandler for LinearWorkflow {
    fn name(&self) -> &str {
        "linear"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("echo build")).await?;
            ctx.shell("test", ShellConfig::new("echo test")).await?;
            ctx.shell("deploy", ShellConfig::new("echo deploy")).await?;
            Ok(())
        })
    }
}

struct ParallelWorkflow;

impl WorkflowHandler for ParallelWorkflow {
    fn name(&self) -> &str {
        "parallel"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("echo build")).await?;
            ctx.parallel(
                vec![
                    ("test-unit", StepConfig::Shell(ShellConfig::new("echo u"))),
                    ("test-int", StepConfig::Shell(ShellConfig::new("echo i"))),
                    ("lint", StepConfig::Shell(ShellConfig::new("echo l"))),
                ],
                true,
            )
            .await?;
            ctx.shell("deploy", ShellConfig::new("echo deploy")).await?;
            Ok(())
        })
    }
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Env {
    Prod,
    Dev,
}

#[derive(Deserialize)]
struct DeployInput {
    env: Env,
}

struct ConditionalWorkflow;

impl WorkflowHandler for ConditionalWorkflow {
    fn name(&self) -> &str {
        "conditional"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            if ctx
                .when("production run", |i: &DeployInput| i.env == Env::Prod)
                .await?
            {
                ctx.shell("deploy-prod", ShellConfig::new("echo prod"))
                    .await?;
            } else {
                ctx.skip("deploy-prod", "not prod").await?;
            }
            Ok(())
        })
    }
}

/// Hands a declared artifact from one step to the next through its handle.
struct ArtifactPipeline;

impl WorkflowHandler for ArtifactPipeline {
    fn name(&self) -> &str {
        "artifact-pipeline"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let build = ctx
                .shell(
                    "build",
                    ShellConfig::new("./gen").output("dist/report.html"),
                )
                .await?;
            let report = build.artifact("report.html")?;
            ctx.shell("publish", ShellConfig::new("./publish").input(&report))
                .await?;
            Ok(())
        })
    }
}

struct DynamicWorkflow;

impl WorkflowHandler for DynamicWorkflow {
    fn name(&self) -> &str {
        "dynamic"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let build = ctx.shell("build", ShellConfig::new("echo build")).await?;
            if ctx.when_dynamic("build succeeded", build.is_success()) {
                ctx.shell("deploy", ShellConfig::new("echo deploy")).await?;
            }
            Ok(())
        })
    }
}

struct GrandChildWorkflow;

impl TypedWorkflow for GrandChildWorkflow {
    type Input = ();
}

impl WorkflowHandler for GrandChildWorkflow {
    fn name(&self) -> &str {
        "grandchild"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("grandchild-step", ShellConfig::new("echo gc"))
                .await?;
            Ok(())
        })
    }
}

struct ChildWorkflow;

impl WorkflowHandler for ChildWorkflow {
    fn name(&self) -> &str {
        "child"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("child-step", ShellConfig::new("echo child"))
                .await?;
            ctx.workflow(&GrandChildWorkflow, ()).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for ChildWorkflow {
    type Input = NestedInput;
}

/// Input of [`ChildWorkflow`].
#[derive(Serialize, Deserialize)]
struct NestedInput {
    nested: bool,
}

struct ParentWorkflow;

impl WorkflowHandler for ParentWorkflow {
    fn name(&self) -> &str {
        "parent"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("parent-step", ShellConfig::new("echo parent"))
                .await?;
            ctx.workflow(&ChildWorkflow, NestedInput { nested: true })
                .await?;
            Ok(())
        })
    }
}

struct SideEffectWorkflow {
    probe: String,
}

impl WorkflowHandler for SideEffectWorkflow {
    fn name(&self) -> &str {
        "side-effect"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("touch", ShellConfig::new(&format!("touch {}", self.probe)))
                .await?;
            Ok(())
        })
    }
}

struct FailingWorkflow;

impl WorkflowHandler for FailingWorkflow {
    fn name(&self) -> &str {
        "failing"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("one", ShellConfig::new("echo one")).await?;
            ctx.shell("two", ShellConfig::new("echo two")).await?;
            Err(EngineError::InvalidWorkflow(
                "handler gave up on purpose".to_string(),
            ))
        })
    }
}

struct ApprovalWorkflow;

impl WorkflowHandler for ApprovalWorkflow {
    fn name(&self) -> &str {
        "approval"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("echo build")).await?;
            ctx.approval("gate", ApprovalConfig::new("Approve deployment?"))
                .await?;
            ctx.delay("cooldown", DelayConfig::from_secs(300)).await?;
            ctx.shell("deploy", ShellConfig::new("echo deploy")).await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plan_lists_sequential_steps_with_their_dependencies() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(LinearWorkflow).unwrap();

        let plan = engine
            .plan_handler("linear", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        assert_eq!(plan.workflow, "linear");
        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["build", "test", "deploy"]);

        assert!(plan.steps[0].depends_on.is_empty());
        assert_eq!(plan.steps[1].depends_on, vec!["build".to_string()]);
        assert_eq!(plan.steps[2].depends_on, vec!["test".to_string()]);

        for step in &plan.steps {
            assert_eq!(step.kind, StepKind::Shell);
            assert_eq!(step.workflow, "linear");
            assert_eq!(step.depth, 0);
            assert!(step.parallel_group.is_none());
            assert!(step.condition.is_none());
        }
        assert!(!plan.truncated);
        assert!(plan.incomplete_reason.is_none());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_groups_a_parallel_wave_and_fans_dependencies_back_in() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(ParallelWorkflow).unwrap();

        let plan = engine
            .plan_handler("parallel", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        assert_eq!(plan.steps.len(), 5);
        let group = plan.steps[1]
            .parallel_group
            .clone()
            .expect("wave member carries a group");
        assert_eq!(group, "parallel-1");
        assert_eq!(
            plan.steps[2].parallel_group.as_deref(),
            Some(group.as_str())
        );
        assert_eq!(
            plan.steps[3].parallel_group.as_deref(),
            Some(group.as_str())
        );

        for member in &plan.steps[1..4] {
            assert_eq!(member.depends_on, vec!["build".to_string()]);
        }

        let deploy = &plan.steps[4];
        assert_eq!(deploy.name, "deploy");
        assert!(deploy.parallel_group.is_none());
        assert_eq!(
            deploy.depends_on,
            vec![
                "test-unit".to_string(),
                "test-int".to_string(),
                "lint".to_string()
            ]
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_evaluates_a_condition_against_the_input() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(ConditionalWorkflow).unwrap();

        let prod = engine
            .plan_handler(
                "conditional",
                json!({"env": "prod"}),
                PlanOptions::default(),
            )
            .await
            .expect("plan built");

        assert_eq!(prod.steps.len(), 1);
        assert_eq!(prod.steps[0].kind, StepKind::Shell);
        match prod.steps[0].condition.as_ref().expect("a condition") {
            ConditionResult::Evaluated { expression, value } => {
                assert_eq!(expression, "production run");
                assert!(*value);
            }
            other => panic!("expected an evaluated condition, got {other:?}"),
        }

        let dev = engine
            .plan_handler("conditional", json!({"env": "dev"}), PlanOptions::default())
            .await
            .expect("plan built");

        assert_eq!(dev.steps.len(), 1);
        assert_eq!(dev.steps[0].kind, StepKind::Custom("skip".to_string()));
        match dev.steps[0].condition.as_ref().expect("a condition") {
            ConditionResult::Skipped { reason } => assert_eq!(reason, "not prod"),
            other => panic!("expected a skipped condition, got {other:?}"),
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_stops_on_a_condition_whose_input_does_not_match_its_type() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(ConditionalWorkflow).unwrap();

        // `staging` is not an `Env`: the typo surfaces instead of a silent false.
        let plan = engine
            .plan_handler(
                "conditional",
                json!({"env": "staging"}),
                PlanOptions::default(),
            )
            .await
            .expect("plan built");

        assert!(plan.steps.is_empty());
        assert!(plan.truncated);
        let reason = plan.incomplete_reason.expect("a reason");
        assert!(reason.contains("staging"), "unexpected reason: {reason}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_run_fails_when_a_condition_input_does_not_match_its_type() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(ConditionalWorkflow).unwrap();

        let err = engine
            .run_handler("conditional", TriggerKind::Manual, json!({"env": 42}))
            .await
            .expect_err("payload does not match DeployInput");

        assert!(
            matches!(err, EngineError::Serialization(_)),
            "unexpected error: {err:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_follows_artifact_handles_without_producing_anything() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(ArtifactPipeline).unwrap();

        let plan = engine
            .plan_handler("artifact-pipeline", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        assert!(
            !plan.truncated,
            "plan stopped: {:?}",
            plan.incomplete_reason
        );
        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["build", "publish"]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_reports_a_step_derived_condition_as_unevaluable() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(DynamicWorkflow).unwrap();

        let plan = engine
            .plan_handler("dynamic", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        assert_eq!(plan.steps.len(), 2);
        assert!(plan.steps[0].condition.is_none());
        match plan.steps[1].condition.as_ref().expect("a condition") {
            ConditionResult::Unevaluable { expression, reason } => {
                assert_eq!(expression, "build succeeded");
                assert!(!reason.is_empty());
            }
            other => panic!("expected an unevaluable condition, got {other:?}"),
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_expands_sub_workflows_recursively() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(ParentWorkflow).unwrap();
        engine.register(ChildWorkflow).unwrap();
        engine.register(GrandChildWorkflow).unwrap();

        let plan = engine
            .plan_handler("parent", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "parent-step",
                "child",
                "child-step",
                "grandchild",
                "grandchild-step"
            ]
        );

        assert_eq!(plan.steps[1].kind, StepKind::Workflow);
        assert_eq!(plan.steps[1].depth, 0);
        assert_eq!(plan.steps[2].depth, 1);
        assert_eq!(plan.steps[2].workflow, "child");
        assert_eq!(plan.steps[4].depth, 2);
        assert_eq!(plan.steps[4].workflow, "grandchild");
        assert!(!plan.truncated);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_stops_expanding_at_the_depth_limit() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(ParentWorkflow).unwrap();
        engine.register(ChildWorkflow).unwrap();
        engine.register(GrandChildWorkflow).unwrap();

        let plan = engine
            .plan_handler(
                "parent",
                json!({}),
                PlanOptions {
                    max_depth: 1,
                    ..PlanOptions::default()
                },
            )
            .await
            .expect("plan built");

        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["parent-step", "child", "child-step", "grandchild"]
        );
        assert!(plan.truncated);
        assert!(
            plan.incomplete_reason
                .expect("a reason")
                .contains("depth 1")
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_runs_nothing_and_persists_nothing() {
    timeout(TEST_TIMEOUT, async {
        let probe = format!("/tmp/ironflow-plan-probe-{}", Uuid::now_v7());
        let store = test_store();
        let mut engine = engine_with(store.clone());
        engine
            .register(SideEffectWorkflow {
                probe: probe.clone(),
            })
            .unwrap();

        let plan = engine
            .plan_handler("side-effect", json!({}), PlanOptions::default())
            .await
            .expect("plan built");
        assert_eq!(plan.steps.len(), 1);

        assert!(
            !Path::new(&probe).exists(),
            "planning must not run the command"
        );
        let _ = remove_file(&probe);

        let runs = store
            .list_runs(RunFilter::default(), 1, 50)
            .await
            .expect("list runs");
        assert!(runs.items.is_empty(), "planning must not create a run");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_returns_a_partial_plan_when_the_handler_fails() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(FailingWorkflow).unwrap();

        let plan = engine
            .plan_handler("failing", json!({}), PlanOptions::default())
            .await
            .expect("a failing handler still yields a plan");

        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["one", "two"]);
        assert!(plan.truncated);
        let reason = plan.incomplete_reason.expect("a reason");
        assert!(reason.contains("handler gave up on purpose"), "{reason}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_does_not_suspend_on_an_approval_gate_or_sleep_on_a_delay() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(ApprovalWorkflow).unwrap();

        let plan = engine
            .plan_handler("approval", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["build", "gate", "cooldown", "deploy"]);
        assert_eq!(plan.steps[1].kind, StepKind::Approval);
        assert_eq!(plan.steps[2].kind, StepKind::Custom("delay".to_string()));
        assert_eq!(
            plan.steps[2].estimated_duration,
            Some(Duration::from_secs(300))
        );
        assert!(!plan.truncated);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_estimates_durations_from_run_history() {
    timeout(TEST_TIMEOUT, async {
        let store = test_store();
        let mut engine = engine_with(store.clone());
        engine.register(LinearWorkflow).unwrap();

        engine
            .run_handler("linear", TriggerKind::Manual, json!({}))
            .await
            .expect("real run succeeds");

        let plan = engine
            .plan_handler("linear", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        for step in &plan.steps {
            assert!(
                step.estimated_duration.is_some(),
                "step {} has no estimate",
                step.name
            );
        }
        assert!(plan.estimated_duration.is_some());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_has_no_estimate_without_history() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(LinearWorkflow).unwrap();

        let plan = engine
            .plan_handler("linear", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        assert!(plan.estimated_duration.is_none());
        for step in &plan.steps {
            assert!(step.estimated_duration.is_none());
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_serves_the_payload_from_the_request_not_from_a_run() {
    timeout(TEST_TIMEOUT, async {
        struct PayloadEchoWorkflow;

        impl WorkflowHandler for PayloadEchoWorkflow {
            fn name(&self) -> &str {
                "payload-echo"
            }

            fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
                Box::pin(async move {
                    let payload: Value = ctx.payload().await?;
                    let name = payload["step"].as_str().unwrap_or("missing").to_string();
                    ctx.shell(&name, ShellConfig::new("echo hi")).await?;
                    Ok(())
                })
            }
        }

        let mut engine = engine_with(test_store());
        engine.register(PayloadEchoWorkflow).unwrap();

        let plan = engine
            .plan_handler(
                "payload-echo",
                json!({"step": "from-input"}),
                PlanOptions::default(),
            )
            .await
            .expect("plan built");

        assert_eq!(plan.steps[0].name, "from-input");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_rejects_an_unknown_workflow() {
    timeout(TEST_TIMEOUT, async {
        let engine = engine_with(test_store());

        let err = engine
            .plan_handler("nope", json!({}), PlanOptions::default())
            .await
            .expect_err("unknown workflow");
        assert!(matches!(err, EngineError::InvalidWorkflow(_)));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_rejects_a_zero_max_depth() {
    timeout(TEST_TIMEOUT, async {
        let mut engine = engine_with(test_store());
        engine.register(LinearWorkflow).unwrap();

        let err = engine
            .plan_handler(
                "linear",
                json!({}),
                PlanOptions {
                    max_depth: 0,
                    ..PlanOptions::default()
                },
            )
            .await
            .expect_err("max_depth must be at least 1");
        assert!(matches!(err, EngineError::InvalidWorkflow(_)));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_skips_estimation_when_asked_to() {
    timeout(TEST_TIMEOUT, async {
        let store = test_store();
        let mut engine = engine_with(store.clone());
        engine.register(LinearWorkflow).unwrap();

        engine
            .run_handler("linear", TriggerKind::Manual, json!({}))
            .await
            .expect("real run succeeds");

        let plan = engine
            .plan_handler(
                "linear",
                json!({}),
                PlanOptions {
                    estimate_durations: false,
                    ..PlanOptions::default()
                },
            )
            .await
            .expect("plan built");

        assert!(plan.estimated_duration.is_none());
    })
    .await
    .expect("test timed out");
}
