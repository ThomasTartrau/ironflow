use std::collections::HashMap;

use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Input payload for the hello-world template.
#[derive(Deserialize, JsonSchema)]
pub struct HelloWorldInput {
    /// Person to greet.
    pub name: String,
    /// Whether to include the current date in the greeting.
    #[serde(default)]
    pub include_date: bool,
}

/// Minimal workflow template that greets someone.
///
/// Steps:
/// 1. Print a greeting
/// 2. Optionally print the current date
pub struct HelloWorld;

impl WorkflowHandler for HelloWorld {
    fn name(&self) -> &str {
        "template/hello-world"
    }

    fn category(&self) -> Option<&str> {
        Some("templates")
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<HelloWorldInput>())
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Minimal workflow that greets someone and logs the current date."
                .to_string(),
            source_code: Some(include_str!("hello_world.rs").to_string()),
            sub_workflows: Vec::new(),
            category: self.category().map(str::to_string),
            version: self.version().map(str::to_string),
            compatible_versions: Vec::new(),
            input_schema: self.input_schema(),
            default_labels: HashMap::new(),
            schedule: None,
            default_max_cost_usd: None,
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let payload = ctx.payload().await?;
            let input: HelloWorldInput = serde_json::from_value(payload)
                .map_err(|e| EngineError::StepConfig(e.to_string()))?;

            ctx.shell(
                "greet",
                ShellConfig::new(&format!("echo 'Hello, {}!'", input.name)),
            )
            .await?;

            if input.include_date {
                ctx.shell("date", ShellConfig::new("date '+%Y-%m-%d %H:%M:%S'"))
                    .await?;
            }

            Ok(())
        })
    }
}
