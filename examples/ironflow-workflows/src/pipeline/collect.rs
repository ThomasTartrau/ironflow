use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};
use std::collections::HashMap;

/// Workflow C: collects raw system metrics (disk, memory, uptime).
pub struct Collect;

impl WorkflowHandler for Collect {
    fn name(&self) -> &str {
        "pipeline-collect"
    }

    fn category(&self) -> Option<&str> {
        Some("examples/pipeline")
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Collects raw system metrics: disk usage, memory, and uptime.".to_string(),
            source_code: Some(include_str!("collect.rs").to_string()),
            sub_workflows: Vec::new(),
            category: Some("examples/pipeline".to_string()),
            version: self.version().map(str::to_string),
            input_schema: None,
            default_labels: HashMap::new(),
            schedule: None,
            default_max_cost_usd: None,
        }
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
