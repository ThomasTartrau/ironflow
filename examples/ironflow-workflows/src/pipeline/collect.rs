use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Workflow C: collects raw system metrics (disk, memory, uptime).
pub struct Collect;

impl WorkflowHandler for Collect {
    fn name(&self) -> &str {
        "pipeline-collect"
    }

    fn category(&self) -> Option<&str> {
        Some("examples/pipeline")
    }

    fn description(&self) -> &str {
        "Collects raw system metrics: disk usage, memory, and uptime."
    }

    fn source_code(&self) -> Option<&str> {
        Some(include_str!("collect.rs"))
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("disk", ShellConfig::new("df -h /")).await?;
            ctx.shell("memory", ShellConfig::new("vm_stat | head -5"))
                .await?;
            ctx.shell("uptime", ShellConfig::new("uptime")).await?;

            Ok(())
        })
    }
}
