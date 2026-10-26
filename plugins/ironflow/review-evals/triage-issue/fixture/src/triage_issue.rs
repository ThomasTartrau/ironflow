use ironflow_engine::config::{AgentStepConfig, HumanInputConfig, Prompt, ShellConfig, Tool};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Input of the `triage-issue` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TriageIssueInput {
    /// Internal id of the issue.
    pub issue_iid: u64,
}

/// Labels proposed by the agent.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Triage {
    /// Labels to put on the issue.
    pub labels: Vec<String>,
    /// One-paragraph summary for the maintainer.
    pub summary: String,
}

/// The maintainer's final say on the labels.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelChoice {
    /// Labels to apply, possibly edited.
    pub labels: Vec<String>,
}

/// Lets an agent propose labels for an issue, a maintainer confirms, labels are applied.
pub struct TriageIssue;

impl WorkflowHandler for TriageIssue {
    fn name(&self) -> &str {
        "triage-issue"
    }

    fn description(&self) -> &str {
        "Propose labels for an issue, have a maintainer confirm them, apply them."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<TriageIssueInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: TriageIssueInput = ctx.input().await?;
            let iid = input.issue_iid.to_string();

            let issue = ctx
                .shell(
                    "fetch-issue",
                    ShellConfig::exec("glab", &["issue", "view", iid.as_str()]),
                )
                .await?;

            let prompt = Prompt::new()
                .text(
                    "Read CONTRIBUTING.md for the label scheme, then propose labels \
                     for this issue:",
                )
                .untrusted(issue.stdout());
            let triage = ctx
                .agent(
                    "propose-labels",
                    AgentStepConfig::new(&prompt.to_string())
                        .allow_tool(Tool::Read)
                        .max_turns(6)
                        .max_budget_usd(0.50)
                        .output::<Triage>(),
                )
                .await?;

            let choice = ctx
                .human_input::<LabelChoice>(
                    "confirm-labels",
                    HumanInputConfig::new(&format!(
                        "{}\n\nProposed labels: {}",
                        triage.summary,
                        triage.labels.join(", ")
                    )),
                )
                .await?;

            let labels = choice.labels.join(",");
            ctx.shell(
                "apply-labels",
                ShellConfig::exec(
                    "glab",
                    &["issue", "update", iid.as_str(), "--label", labels.as_str()],
                ),
            )
            .await?;

            Ok(())
        })
    }
}
