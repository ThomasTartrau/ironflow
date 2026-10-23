use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Empties the build cache every night.
pub struct PurgeCache;

impl WorkflowHandler for PurgeCache {
    fn name(&self) -> &str {
        "nightly-maintenance"
    }

    fn description(&self) -> &str {
        "Empty the build cache."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("purge", ShellConfig::new("rm -rf /var/cache/build/*"))
                .await?;
            Ok(())
        })
    }
}
