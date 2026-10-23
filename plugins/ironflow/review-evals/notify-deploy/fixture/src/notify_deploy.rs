use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Input of the `notify-deploy` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NotifyDeployInput {
    /// Service to deploy, as named in the deploy script.
    pub service: String,
}

/// Announces a deploy in the team channel, builds, waits for approval, deploys.
pub struct NotifyDeploy;

impl WorkflowHandler for NotifyDeploy {
    fn name(&self) -> &str {
        "notify-deploy"
    }

    fn description(&self) -> &str {
        "Announce the deploy, build, wait for approval, deploy."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<NotifyDeployInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: NotifyDeployInput = ctx.input().await?;

            let client = Client::new();
            client
                .post("https://chat.example.com/hooks/deploys")
                .json(&json!({ "text": format!("{} is waiting for a deploy approval", input.service) }))
                .send()
                .await
                .map_err(|e| EngineError::StepConfig(e.to_string()))?;

            ctx.shell(
                "build",
                ShellConfig::new("cargo build --release").env("SERVICE", &input.service),
            )
            .await?;

            ctx.approval(
                "approve-deploy",
                ApprovalConfig::new("Deploy to production?"),
            )
            .await?;

            ctx.shell(
                "deploy",
                ShellConfig::new("./deploy.sh \"$SERVICE\"").env("SERVICE", &input.service),
            )
            .await?;

            Ok(())
        })
    }
}
