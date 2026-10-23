use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Dumps the main database and uploads the dump to the backup bucket.
pub struct BackupDb;

impl WorkflowHandler for BackupDb {
    fn name(&self) -> &str {
        "backup-db"
    }

    fn description(&self) -> &str {
        "Dump the main database and upload it to the backup bucket."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("dump", ShellConfig::new("./scripts/pg-dump.sh"))
                .await?;
            ctx.shell("upload", ShellConfig::new("./scripts/upload-dump.sh"))
                .await?;
            Ok(())
        })
    }
}
