use ironflow_engine::config::AgentStepConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::executor::StepOutput;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler, sub_workflow_names};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Collect;

/// Name of the agent step that structures the metrics. [`Report`](super::Report)
/// reads that step back from the store.
pub const STRUCTURE_STEP: &str = "structure";

/// System metrics, as the agent reads them from the raw command output.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SystemMetrics {
    /// Used share of the root disk, in percent.
    pub disk_usage_percent: f64,
    /// Free memory pages reported by `vm_stat`.
    pub memory_free_pages: u64,
    /// Time since boot, in days.
    pub uptime_days: f64,
}

/// Workflow B: runs the collect sub-workflow, then enriches the raw data
/// with an AI-generated structured summary.
pub struct Enrich;

impl WorkflowHandler for Enrich {
    fn name(&self) -> &str {
        "pipeline-enrich"
    }

    fn category(&self) -> Option<&str> {
        Some("examples/pipeline")
    }

    fn description(&self) -> &str {
        "Calls pipeline-collect to gather raw metrics, then uses an AI agent \
         to parse them into structured system metrics."
    }

    fn source_code(&self) -> Option<&str> {
        Some(include_str!("enrich.rs"))
    }

    fn sub_workflows(&self) -> Vec<String> {
        sub_workflow_names(&[&Collect])
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let collect = ctx.workflow(&Collect, ()).await?;

            let raw_data = ctx
                .store()
                .list_steps(collect.run_id())
                .await?
                .iter()
                .map(|step| (step.name.clone(), StepOutput::from(step)))
                .filter(|(_, output)| !output.stdout().is_empty())
                .map(|(name, output)| format!("=== {name} ===\n{}", output.stdout()))
                .collect::<Vec<_>>()
                .join("\n\n");

            // The typed answer is persisted as the step output, where Report
            // reads it back.
            ctx.agent(
                STRUCTURE_STEP,
                AgentStepConfig::new(&format!(
                    "Here is raw system output:\n\n{raw_data}\n\n\
                     Extract the disk usage percentage of `/`, the number of \
                     free memory pages and the uptime in days."
                ))
                .model("haiku")
                .max_budget_usd(0.10)
                .max_turns(2)
                .output::<SystemMetrics>(),
            )
            .await?;

            Ok(())
        })
    }
}

/// Called as a sub-workflow by [`Report`](super::Report), with no input.
impl TypedWorkflow for Enrich {
    type Input = ();
}
