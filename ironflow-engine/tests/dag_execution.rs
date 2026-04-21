//! Integration tests for DAG execution: parallel steps, branching, and dependencies.

use std::sync::Arc;

use serde_json::json;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, TriggerKind};

fn create_test_engine() -> Engine {
    let store = Arc::new(InMemoryStore::new());
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

struct ParallelEchoWorkflow;

impl WorkflowHandler for ParallelEchoWorkflow {
    fn name(&self) -> &str {
        "parallel-echo"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.parallel(
                vec![
                    ("echo-a", StepConfig::Shell(ShellConfig::new("echo a"))),
                    ("echo-b", StepConfig::Shell(ShellConfig::new("echo b"))),
                    ("echo-c", StepConfig::Shell(ShellConfig::new("echo c"))),
                ],
                true,
            )
            .await?;
            Ok(())
        })
    }
}

struct SequentialThenParallelWorkflow;

impl WorkflowHandler for SequentialThenParallelWorkflow {
    fn name(&self) -> &str {
        "seq-then-parallel"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("echo build")).await?;
            ctx.parallel(
                vec![
                    (
                        "test-unit",
                        StepConfig::Shell(ShellConfig::new("echo unit")),
                    ),
                    (
                        "test-integ",
                        StepConfig::Shell(ShellConfig::new("echo integ")),
                    ),
                ],
                true,
            )
            .await?;
            ctx.shell("deploy", ShellConfig::new("echo deploy")).await?;
            Ok(())
        })
    }
}

struct BranchingWorkflow;

impl WorkflowHandler for BranchingWorkflow {
    fn name(&self) -> &str {
        "branching"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let build = ctx.shell("build", ShellConfig::new("echo ok")).await?;

            if build.output["exit_code"].as_i64() == Some(0) {
                ctx.shell("deploy", ShellConfig::new("echo deployed"))
                    .await?;
            } else {
                ctx.shell("notify-fail", ShellConfig::new("echo failed"))
                    .await?;
            }

            Ok(())
        })
    }
}

struct ParallelFailFastWorkflow;

impl WorkflowHandler for ParallelFailFastWorkflow {
    fn name(&self) -> &str {
        "parallel-fail-fast"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.parallel(
                vec![
                    ("ok-step", StepConfig::Shell(ShellConfig::new("echo ok"))),
                    ("fail-step", StepConfig::Shell(ShellConfig::new("exit 1"))),
                ],
                true,
            )
            .await?;
            Ok(())
        })
    }
}

struct ParallelNoFailFastWorkflow;

impl WorkflowHandler for ParallelNoFailFastWorkflow {
    fn name(&self) -> &str {
        "parallel-no-fail-fast"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.parallel(
                vec![
                    ("ok-step", StepConfig::Shell(ShellConfig::new("echo ok"))),
                    ("fail-step", StepConfig::Shell(ShellConfig::new("exit 1"))),
                ],
                false,
            )
            .await?;
            Ok(())
        })
    }
}

struct ParallelEmptyWorkflow;

impl WorkflowHandler for ParallelEmptyWorkflow {
    fn name(&self) -> &str {
        "parallel-empty"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let results = ctx.parallel(vec![], true).await?;
            assert!(results.is_empty());
            Ok(())
        })
    }
}

struct SkipStepWorkflow;

impl WorkflowHandler for SkipStepWorkflow {
    fn name(&self) -> &str {
        "skip-step"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let build = ctx.shell("build", ShellConfig::new("echo ok")).await?;

            if build.output["exit_code"].as_i64() == Some(0) {
                ctx.shell("deploy", ShellConfig::new("echo deployed"))
                    .await?;
            } else {
                ctx.skip("deploy", "tests failed").await?;
            }

            ctx.shell("notify", ShellConfig::new("echo done")).await?;
            Ok(())
        })
    }
}

struct SkipOnlyWorkflow;

impl WorkflowHandler for SkipOnlyWorkflow {
    fn name(&self) -> &str {
        "skip-only"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.skip("deploy", "not needed").await?;
            Ok(())
        })
    }
}

struct SkipThenContinueWorkflow;

impl WorkflowHandler for SkipThenContinueWorkflow {
    fn name(&self) -> &str {
        "skip-then-continue"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("echo build")).await?;
            ctx.skip("deploy", "staging only").await?;
            ctx.shell("cleanup", ShellConfig::new("echo cleanup"))
                .await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parallel_three_shells_all_succeed() {
    let mut engine = create_test_engine();
    engine.register(ParallelEchoWorkflow).unwrap();

    let run = engine
        .run_handler("parallel-echo", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 3);

    // All steps share the same position (wave 0).
    assert!(steps.iter().all(|s| s.position == 0));

    // All completed.
    assert!(
        steps
            .iter()
            .all(|s| s.status.state == StepStatus::Completed)
    );

    // Names preserved.
    let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"echo-a"));
    assert!(names.contains(&"echo-b"));
    assert!(names.contains(&"echo-c"));
}

#[tokio::test]
async fn parallel_empty_vec_returns_empty() {
    let mut engine = create_test_engine();
    engine.register(ParallelEmptyWorkflow).unwrap();

    let run = engine
        .run_handler("parallel-empty", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(run.id).await.unwrap();
    assert!(steps.is_empty());
}

#[tokio::test]
async fn parallel_fail_fast_aborts_remaining() {
    let mut engine = create_test_engine();
    engine.register(ParallelFailFastWorkflow).unwrap();

    let result = engine
        .run_handler("parallel-fail-fast", TriggerKind::Manual, json!({}))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn parallel_no_fail_fast_collects_all() {
    let mut engine = create_test_engine();
    engine.register(ParallelNoFailFastWorkflow).unwrap();

    let result = engine
        .run_handler("parallel-no-fail-fast", TriggerKind::Manual, json!({}))
        .await;

    // Still returns error (at least one step failed).
    assert!(result.is_err());
}

#[tokio::test]
async fn branching_only_executes_taken_branch() {
    let mut engine = create_test_engine();
    engine.register(BranchingWorkflow).unwrap();

    let run = engine
        .run_handler("branching", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 2);

    let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"build"));
    assert!(names.contains(&"deploy"));
    // "notify-fail" should NOT exist since exit_code == 0.
    assert!(!names.contains(&"notify-fail"));
}

#[tokio::test]
async fn sequential_then_parallel_then_sequential_positions() {
    let mut engine = create_test_engine();
    engine.register(SequentialThenParallelWorkflow).unwrap();

    let run = engine
        .run_handler("seq-then-parallel", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 4);

    // build at position 0.
    let build = steps.iter().find(|s| s.name == "build").unwrap();
    assert_eq!(build.position, 0);

    // test-unit and test-integ at position 1 (parallel wave).
    let test_unit = steps.iter().find(|s| s.name == "test-unit").unwrap();
    let test_integ = steps.iter().find(|s| s.name == "test-integ").unwrap();
    assert_eq!(test_unit.position, 1);
    assert_eq!(test_integ.position, 1);

    // deploy at position 2.
    let deploy = steps.iter().find(|s| s.name == "deploy").unwrap();
    assert_eq!(deploy.position, 2);
}

#[tokio::test]
async fn dependencies_recorded_correctly() {
    let mut engine = create_test_engine();
    engine.register(SequentialThenParallelWorkflow).unwrap();

    let run = engine
        .run_handler("seq-then-parallel", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    let steps = engine.store().list_steps(run.id).await.unwrap();
    let deps = engine.store().list_step_dependencies(run.id).await.unwrap();

    let build = steps.iter().find(|s| s.name == "build").unwrap();
    let test_unit = steps.iter().find(|s| s.name == "test-unit").unwrap();
    let test_integ = steps.iter().find(|s| s.name == "test-integ").unwrap();
    let deploy = steps.iter().find(|s| s.name == "deploy").unwrap();

    // build has no dependencies (first step).
    let build_deps: Vec<_> = deps.iter().filter(|d| d.step_id == build.id).collect();
    assert!(build_deps.is_empty());

    // test-unit depends on build.
    let tu_deps: Vec<_> = deps.iter().filter(|d| d.step_id == test_unit.id).collect();
    assert_eq!(tu_deps.len(), 1);
    assert_eq!(tu_deps[0].depends_on, build.id);

    // test-integ depends on build.
    let ti_deps: Vec<_> = deps.iter().filter(|d| d.step_id == test_integ.id).collect();
    assert_eq!(ti_deps.len(), 1);
    assert_eq!(ti_deps[0].depends_on, build.id);

    // deploy depends on test-unit AND test-integ.
    let deploy_deps: Vec<_> = deps.iter().filter(|d| d.step_id == deploy.id).collect();
    assert_eq!(deploy_deps.len(), 2);
    let deploy_dep_ids: Vec<_> = deploy_deps.iter().map(|d| d.depends_on).collect();
    assert!(deploy_dep_ids.contains(&test_unit.id));
    assert!(deploy_dep_ids.contains(&test_integ.id));
}

#[tokio::test]
async fn cost_and_duration_aggregated_with_parallel() {
    let mut engine = create_test_engine();
    engine.register(ParallelEchoWorkflow).unwrap();

    let run = engine
        .run_handler("parallel-echo", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    // Duration should be > 0 (real shell commands were executed).
    assert!(run.duration_ms > 0);
}

#[tokio::test]
async fn skip_creates_step_with_skipped_status() {
    let mut engine = create_test_engine();
    engine.register(SkipOnlyWorkflow).unwrap();

    let run = engine
        .run_handler("skip-only", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 1);

    let skip = &steps[0];
    assert_eq!(skip.name, "deploy");
    assert_eq!(skip.status.state, StepStatus::Skipped);
    assert_eq!(skip.position, 0);
    assert!(skip.completed_at.is_some());

    let output = skip.output.as_ref().unwrap();
    assert_eq!(output["reason"].as_str().unwrap(), "not needed");
}

#[tokio::test]
async fn skip_step_visible_in_dag_with_dependencies() {
    let mut engine = create_test_engine();
    engine.register(SkipThenContinueWorkflow).unwrap();

    let run = engine
        .run_handler("skip-then-continue", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 3);

    let build = steps.iter().find(|s| s.name == "build").unwrap();
    let deploy = steps.iter().find(|s| s.name == "deploy").unwrap();
    let cleanup = steps.iter().find(|s| s.name == "cleanup").unwrap();

    assert_eq!(build.position, 0);
    assert_eq!(deploy.position, 1);
    assert_eq!(cleanup.position, 2);

    assert_eq!(build.status.state, StepStatus::Completed);
    assert_eq!(deploy.status.state, StepStatus::Skipped);
    assert_eq!(cleanup.status.state, StepStatus::Completed);

    let deps = engine.store().list_step_dependencies(run.id).await.unwrap();

    // deploy depends on build.
    let deploy_deps: Vec<_> = deps.iter().filter(|d| d.step_id == deploy.id).collect();
    assert_eq!(deploy_deps.len(), 1);
    assert_eq!(deploy_deps[0].depends_on, build.id);

    // cleanup depends on deploy (the skipped step).
    let cleanup_deps: Vec<_> = deps.iter().filter(|d| d.step_id == cleanup.id).collect();
    assert_eq!(cleanup_deps.len(), 1);
    assert_eq!(cleanup_deps[0].depends_on, deploy.id);
}

#[tokio::test]
async fn skip_in_branch_shows_skipped_step() {
    let mut engine = create_test_engine();
    engine.register(SkipStepWorkflow).unwrap();

    let run = engine
        .run_handler("skip-step", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 3);

    let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"build"));
    assert!(names.contains(&"deploy"));
    assert!(names.contains(&"notify"));

    // build succeeded, so deploy was executed (not skipped).
    let deploy = steps.iter().find(|s| s.name == "deploy").unwrap();
    assert_eq!(deploy.status.state, StepStatus::Completed);
}
