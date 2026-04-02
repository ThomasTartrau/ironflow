//! Demo workflow showcasing parallel execution and conditional branching.

use ironflow_engine::config::{ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};

/// Simulated CI pipeline that demonstrates DAG features:
///
/// 1. **build** (sequential)
/// 2. **test-unit + test-integration + lint** (parallel)
/// 3. **deploy** or **notify-failure** (conditional branch on test results)
pub struct CiPipeline;

impl WorkflowHandler for CiPipeline {
    fn name(&self) -> &str {
        "ci-pipeline"
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Simulated CI pipeline with parallel tests and conditional deploy. \
                          Demonstrates ctx.parallel() and native Rust if/else branching."
                .to_string(),
            source_code: Some(include_str!("ci_pipeline.rs").to_string()),
            sub_workflows: Vec::new(),
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Step 1: Build
            let build = ctx
                .shell(
                    "build",
                    ShellConfig::new(
                        "echo 'Compiling project...' && sleep 0.2 && echo 'Build successful'",
                    ),
                )
                .await?;

            let build_ok = build
                .output
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                == 0;

            if !build_ok {
                ctx.shell(
                    "notify-build-failure",
                    ShellConfig::new("echo 'BUILD FAILED - notifying team'"),
                )
                .await?;
                return Ok(());
            }

            // Step 2: Parallel tests + lint
            let results = ctx
                .parallel(
                    vec![
                        (
                            "test-unit",
                            StepConfig::Shell(ShellConfig::new(
                                "echo 'Running unit tests...' && sleep 0.3 && echo '42 tests passed'",
                            )),
                        ),
                        (
                            "test-integration",
                            StepConfig::Shell(ShellConfig::new(
                                "echo 'Running integration tests...' && sleep 0.5 && echo '12 tests passed'",
                            )),
                        ),
                        (
                            "lint",
                            StepConfig::Shell(ShellConfig::new(
                                "echo 'Running linter...' && sleep 0.1 && echo 'No warnings'",
                            )),
                        ),
                    ],
                    true,
                )
                .await?;

            // Step 3: Conditional deploy
            let all_passed = results.iter().all(|r| {
                r.output
                    .output
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    == 0
            });

            if all_passed {
                ctx.shell(
                    "deploy",
                    ShellConfig::new("echo 'Deploying to production...' && sleep 0.2 && echo 'Deployed successfully'"),
                )
                .await?;
            } else {
                ctx.shell(
                    "notify-test-failure",
                    ShellConfig::new("echo 'TESTS FAILED - deployment skipped'"),
                )
                .await?;
            }

            Ok(())
        })
    }
}
