use ironflow_engine::config::{AgentStepConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};

pub struct SystemAudit;

impl WorkflowHandler for SystemAudit {
    fn name(&self) -> &str {
        "system-audit"
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Collects system diagnostics (disk, memory, uptime) via shell \
                          commands, then asks an AI agent to analyze health status."
                .to_string(),
            source_code: Some(include_str!("system_audit.rs").to_string()),
            sub_workflows: Vec::new(),
            category: None,
            version: self.version().to_string(),
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Step 1-3: Collect system diagnostics
            let disk = ctx.shell("disk-usage", ShellConfig::new("df -h /")).await?;
            let memory = ctx
                .shell("memory", ShellConfig::new("vm_stat | head -5"))
                .await?;
            let uptime = ctx.shell("uptime", ShellConfig::new("uptime")).await?;

            // Step 4: AI summary of all diagnostics
            let disk_out = disk
                .output
                .get("stdout")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mem_out = memory
                .output
                .get("stdout")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let up_out = uptime
                .output
                .get("stdout")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            ctx.agent(
                "analysis",
                AgentStepConfig::new(&format!(
                    "Analyze this system diagnostic data and provide a health summary:\n\n\
                     Disk Usage:\n{disk_out}\n\n\
                     Memory:\n{mem_out}\n\n\
                     Uptime:\n{up_out}\n\n\
                     Provide: overall health status (healthy/warning/critical), \
                     key observations, and any recommendations. Be concise."
                ))
                .model("haiku")
                .max_budget_usd(0.05)
                .max_turns(1),
            )
            .await?;

            Ok(())
        })
    }
}
