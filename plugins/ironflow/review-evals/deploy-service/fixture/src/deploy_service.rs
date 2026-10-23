use ironflow_engine::config::{ApprovalConfig, HttpConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Input of the `deploy-service` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeployServiceInput {
    /// Service to deploy, as named in the deploy script.
    pub service: String,
    /// Target environment.
    #[serde(default = "default_environment")]
    pub environment: String,
}

fn default_environment() -> String {
    "staging".to_string()
}

/// Builds a service, announces the deploy, waits for approval in production, deploys.
pub struct DeployService;

impl WorkflowHandler for DeployService {
    fn name(&self) -> &str {
        "deploy-service"
    }

    fn description(&self) -> &str {
        "Build a service and deploy it, with an approval gate in production."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<DeployServiceInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: DeployServiceInput = ctx.input().await?;

            ctx.shell(
                "build",
                ShellConfig::new("cargo build --release -p \"$SERVICE\"")
                    .env("SERVICE", &input.service),
            )
            .await?;

            ctx.http(
                "announce",
                HttpConfig::post("https://chat.example.com/hooks/deploys").json(json!({
                    "text": format!("{} is ready for {}", input.service, input.environment),
                })),
            )
            .await?;

            if ctx
                .when("production run", |i: &DeployServiceInput| {
                    i.environment == "production"
                })
                .await?
            {
                ctx.approval(
                    "approve-production",
                    ApprovalConfig::new("Deploy to production?"),
                )
                .await?;
            }

            let registry = ctx
                .secrets()?
                .get("registry_password")
                .await
                .map_err(EngineError::Store)?
                .ok_or_else(|| {
                    EngineError::StepConfig("secret registry_password missing".to_string())
                })?;

            ctx.shell(
                "deploy",
                ShellConfig::exec(
                    "./deploy.sh",
                    &[input.service.as_str(), input.environment.as_str()],
                )
                .env("REGISTRY_PASSWORD", &registry.value),
            )
            .await?;

            Ok(())
        })
    }
}
