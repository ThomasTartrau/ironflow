use ironflow_core::operations::agent::Model;
use ironflow_engine::config::{AgentStepConfig, Prompt, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Input of the `pr-summary` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrSummaryInput {
    /// Internal id of the merge request.
    pub mr_iid: u64,
}

/// Summarises a merge request diff and posts the summary as a note.
pub struct PrSummary;

impl WorkflowHandler for PrSummary {
    fn name(&self) -> &str {
        "pr-summary"
    }

    fn description(&self) -> &str {
        "Summarise a merge request and post the summary on it."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<PrSummaryInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: PrSummaryInput = ctx.input().await?;
            let iid = input.mr_iid.to_string();

            let diff = ctx
                .shell(
                    "diff",
                    ShellConfig::exec("glab", &["mr", "diff", iid.as_str()]),
                )
                .await?;

            let prompt = Prompt::new()
                .text("Summarise this merge request diff in three bullet points:")
                .untrusted(diff.stdout());
            let summary = ctx
                .agent(
                    "summarize",
                    AgentStepConfig::new(&prompt.to_string())
                        .model(Model::HAIKU)
                        .max_turns(1)
                        .max_budget_usd(0.10),
                )
                .await?;

            ctx.shell(
                "post-note",
                ShellConfig::new(&format!(
                    "glab mr note {} --message \"{}\"",
                    input.mr_iid,
                    summary.text()
                )),
            )
            .await?;

            Ok(())
        })
    }
}
