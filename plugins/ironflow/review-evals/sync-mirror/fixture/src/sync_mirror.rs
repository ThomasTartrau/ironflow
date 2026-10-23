use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use tracing::info;

/// Pushes the internal repository to its public GitHub mirror.
pub struct SyncMirror;

impl WorkflowHandler for SyncMirror {
    fn name(&self) -> &str {
        "sync-mirror"
    }

    fn description(&self) -> &str {
        "Push the internal repository to the public GitHub mirror."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let token = ctx
                .secrets()?
                .get("github_mirror_token")
                .await
                .map_err(EngineError::Store)?
                .ok_or_else(|| {
                    EngineError::StepConfig("secret github_mirror_token missing".to_string())
                })?;

            info!("pushing mirror with token {}", token.value);

            ctx.shell(
                "push-mirror",
                ShellConfig::new("./scripts/push-mirror.sh").env("GITHUB_TOKEN", &token.value),
            )
            .await?;

            Ok(())
        })
    }
}
