use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Tests the crate, waits for a maintainer, publishes it to crates.io.
pub struct PublishCrate;

impl WorkflowHandler for PublishCrate {
    fn name(&self) -> &str {
        "publish-crate"
    }

    fn description(&self) -> &str {
        "Test the crate, then publish it to crates.io after approval."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("test", ShellConfig::new("cargo test --all-features"))
                .await?;

            ctx.approval(
                "approve-publish",
                ApprovalConfig::new("Publish this version to crates.io?"),
            )
            .await?;

            let token = ctx
                .secrets()?
                .get("crates_io_token")
                .await
                .map_err(EngineError::Store)?
                .ok_or_else(|| {
                    EngineError::StepConfig("secret crates_io_token missing".to_string())
                })?;

            ctx.shell(
                "cargo-publish",
                ShellConfig::new(&format!("cargo publish --token {}", token.value)),
            )
            .await?;

            Ok(())
        })
    }
}
