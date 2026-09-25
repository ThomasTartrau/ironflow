use ironflow_engine::config::AgentStepConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::executor::StepOutput;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, sub_workflow_names};

use super::Enrich;
use super::enrich::{STRUCTURE_STEP, SystemMetrics};

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

    fn description(&self) -> &str {
        "Full system report pipeline: collect → enrich → report. \
         Calls pipeline-enrich (which calls pipeline-collect), \
         then produces a human-readable report."
    }

    fn source_code(&self) -> Option<&str> {
        Some(include_str!("report.rs"))
    }

    fn sub_workflows(&self) -> Vec<String> {
        sub_workflow_names(&[&Enrich])
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let enrich = ctx.workflow(&Enrich, ()).await?;

            // While planning no child run exists, so there is nothing to read.
            let steps = ctx.store().list_steps(enrich.run_id()).await?;
            let summary = match steps.iter().find(|step| step.name == STRUCTURE_STEP) {
                Some(step) => {
                    let metrics: SystemMetrics = StepOutput::from(step).json()?;
                    format!(
                        "disk usage {:.0}%, {} free memory pages, uptime {:.1} days",
                        metrics.disk_usage_percent, metrics.memory_free_pages, metrics.uptime_days
                    )
                }
                None => "no metrics were collected".to_string(),
            };

            ctx.agent(
                "final-report",
                AgentStepConfig::new(&format!(
                    "Here is the system data: {summary}.\n\n\
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
