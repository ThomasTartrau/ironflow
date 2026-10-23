use chrono::Utc;
use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Builds and packages the release archive, then publishes it after approval.
pub struct BuildMatrix;

impl WorkflowHandler for BuildMatrix {
    fn name(&self) -> &str {
        "build-matrix"
    }

    fn description(&self) -> &str {
        "Build, test and package a release archive, publish it after approval."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("cargo build --release"))
                .await?;
            ctx.shell("test", ShellConfig::new("cargo test --release"))
                .await?;

            let stamp = Utc::now().format("%Y%m%d%H%M").to_string();
            ctx.shell(
                &format!("package-{stamp}"),
                ShellConfig::new("tar czf \"dist/app-$STAMP.tar.gz\" target/release/app")
                    .env("STAMP", &stamp),
            )
            .await?;

            ctx.approval(
                "publish-archive",
                ApprovalConfig::new("Publish the release archive?"),
            )
            .await?;

            ctx.shell(
                "publish",
                ShellConfig::new("./scripts/upload.sh \"dist/app-$STAMP.tar.gz\"")
                    .env("STAMP", &stamp),
            )
            .await?;

            Ok(())
        })
    }
}
