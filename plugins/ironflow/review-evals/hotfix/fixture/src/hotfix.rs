use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Input of the `hotfix` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HotfixInput {
    /// Ticket the hotfix answers, such as `OPS-1234`.
    pub ticket: String,
}

/// Opens a hotfix branch, pages the on-call lead, and opens the merge request once approved.
pub struct Hotfix;

impl WorkflowHandler for Hotfix {
    fn name(&self) -> &str {
        "hotfix"
    }

    fn description(&self) -> &str {
        "Open a hotfix branch and its merge request after the on-call lead approves."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<HotfixInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: HotfixInput = ctx.input().await?;

            ctx.shell(
                "branch",
                ShellConfig::new(&format!(
                    "git checkout -b hotfix/{} origin/main && git push -u origin HEAD",
                    input.ticket
                ))
                .dir("/srv/repo"),
            )
            .await?;

            Client::new()
                .post("https://oncall.example.com/api/page")
                .json(&json!({ "summary": format!("Hotfix {} needs a review", input.ticket) }))
                .send()
                .await
                .map_err(|e| EngineError::StepConfig(e.to_string()))?;

            ctx.approval(
                "approve-hotfix",
                ApprovalConfig::new("Open the hotfix merge request?"),
            )
            .await?;

            let token = ctx
                .secrets()?
                .get("gitlab_token")
                .await
                .map_err(EngineError::Store)?
                .ok_or_else(|| {
                    EngineError::StepConfig("secret gitlab_token missing".to_string())
                })?;

            ctx.shell(
                "open-mr",
                ShellConfig::new(&format!(
                    "curl -sf -X POST -H 'PRIVATE-TOKEN: {}' \
                     'https://gitlab.example.com/api/v4/projects/42/merge_requests' \
                     -d source_branch=\"$BRANCH\" -d target_branch=main -d title=\"$BRANCH\"",
                    token.value
                ))
                .env("BRANCH", &format!("hotfix/{}", input.ticket)),
            )
            .await?;

            Ok(())
        })
    }
}
