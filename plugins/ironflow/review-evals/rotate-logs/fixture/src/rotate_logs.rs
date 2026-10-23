use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Compresses yesterday's application logs and drops the ones older than 30 days.
pub struct RotateLogs;

impl WorkflowHandler for RotateLogs {
    fn name(&self) -> &str {
        "nightly-maintenance"
    }

    fn description(&self) -> &str {
        "Compress yesterday's logs and drop the ones older than 30 days."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell(
                "compress",
                ShellConfig::new("find /var/log/app -name '*.log' -mtime +0 -exec gzip {} +"),
            )
            .await?;
            ctx.shell(
                "expire",
                ShellConfig::new("find /var/log/app -name '*.log.gz' -mtime +30 -delete"),
            )
            .await?;
            Ok(())
        })
    }
}
