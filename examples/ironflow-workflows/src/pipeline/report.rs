use ironflow_engine::config::AgentStepConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};
use serde_json::json;

use super::Enrich;

/// Workflow A: orchestrates the full pipeline.
///
/// Calls pipeline-enrich (which itself calls pipeline-collect),
/// then generates a final human-readable report from the structured data.
pub struct Report;

impl WorkflowHandler for Report {
    fn name(&self) -> &str {
        "pipeline-report"
    }

    fn category(&self) -> Option<&str> {
        Some("examples/pipeline")
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Full system report pipeline: collect → enrich → report. \
                          Calls pipeline-enrich (which calls pipeline-collect), \
                          then produces a human-readable report."
                .to_string(),
            source_code: Some(include_str!("report.rs").to_string()),
            sub_workflows: vec!["pipeline-enrich".to_string()],
            category: Some("examples/pipeline".to_string()),
            version: self.version().to_string(),
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let enrich_result = ctx.workflow(&Enrich, json!({})).await?;

            let child_run_id = enrich_result
                .output
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let steps = ctx
                .store()
                .list_steps(child_run_id.parse().unwrap_or_default())
                .await?;

            let structured_data = steps
                .iter()
                .find(|s| s.name == "structure")
                .and_then(|s| s.output.as_ref())
                .and_then(|o| o.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}");

            ctx.agent(
                "final-report",
                AgentStepConfig::new(&format!(
                    "Here is structured system data:\n\n{structured_data}\n\n\
                     Write a brief, friendly system health report (5 lines max). \
                     Mention disk usage, memory, and uptime. \
                     Add a recommendation if anything looks concerning."
                ))
                .model("haiku")
                .max_budget_usd(0.10)
                .max_turns(1),
            )
            .await?;

            Ok(())
        })
    }
}
