use ironflow_engine::config::{AgentStepConfig, ShellConfig, Tool};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Input of the `repair-pipeline` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RepairPipelineInput {
    /// Id of the failed CI job.
    pub job_id: u64,
}

/// Reads the log of a failed CI job and lets an agent fix the build.
pub struct RepairPipeline;

impl WorkflowHandler for RepairPipeline {
    fn name(&self) -> &str {
        "repair-pipeline"
    }

    fn description(&self) -> &str {
        "Fix the build from the log of a failed CI job."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<RepairPipelineInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: RepairPipelineInput = ctx.input().await?;
            let job = input.job_id.to_string();

            let log = ctx
                .shell(
                    "job-log",
                    ShellConfig::exec("glab", &["ci", "trace", job.as_str()]),
                )
                .await?;

            ctx.agent(
                "fix-build",
                AgentStepConfig::new(&format!(
                    "The CI job failed with this log. Fix the code so the build passes:\n\n{}",
                    log.stdout()
                ))
                .allow_tool(Tool::Bash)
                .allow_tool(Tool::Edit)
                .max_turns(20)
                .max_budget_usd(2.00),
            )
            .await?;

            Ok(())
        })
    }
}
