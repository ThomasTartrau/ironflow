use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Runs the test suite and only proposes a release when it is green.
pub struct ReleaseGate;

impl WorkflowHandler for ReleaseGate {
    fn name(&self) -> &str {
        "release-gate"
    }

    fn description(&self) -> &str {
        "Run the tests, and propose a release only when they pass."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let tests = ctx
                .shell(
                    "tests",
                    ShellConfig::new("cargo test --workspace").exit_code_as_output(),
                )
                .await?;

            if tests.output["exitCode"].as_i64() != Some(0) {
                return Ok(());
            }

            ctx.approval(
                "approve-release",
                ApprovalConfig::new("Tests are green. Tag a release?"),
            )
            .await?;

            ctx.shell("tag", ShellConfig::new("./scripts/tag-release.sh"))
                .await?;

            Ok(())
        })
    }
}
