use std::collections::HashMap;

use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Input payload for the greeting workflow.
///
/// Derives [`JsonSchema`] so the dashboard can render a dynamic form.
#[derive(Deserialize, JsonSchema)]
struct GreetingInput {
    /// Person to greet.
    name: String,
    /// Greeting language (en, fr, es).
    #[serde(default = "default_language")]
    language: String,
    /// Number of times to repeat the greeting.
    #[serde(default = "default_repeat")]
    repeat: u32,
    /// Whether to output in uppercase.
    #[serde(default)]
    uppercase: bool,
}

fn default_language() -> String {
    "en".to_string()
}

fn default_repeat() -> u32 {
    1
}

pub struct Greeting;

impl WorkflowHandler for Greeting {
    fn name(&self) -> &str {
        "greeting"
    }

    fn category(&self) -> Option<&str> {
        Some("examples")
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<GreetingInput>())
    }

    fn default_labels(&self) -> HashMap<String, String> {
        HashMap::from([("project".to_string(), "ironflow".to_string())])
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "A demo workflow that greets someone. \
                          Shows how input_schema generates a dynamic form in the dashboard."
                .to_string(),
            source_code: Some(include_str!("greeting.rs").to_string()),
            sub_workflows: Vec::new(),
            category: self.category().map(str::to_string),
            version: self.version().map(str::to_string),
            input_schema: self.input_schema(),
            default_labels: self.default_labels(),
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let payload = ctx.payload().await?;
            let input: GreetingInput = serde_json::from_value(payload)
                .map_err(|e| ironflow_engine::error::EngineError::StepConfig(e.to_string()))?;

            let greeting = match input.language.as_str() {
                "fr" => format!("Bonjour, {} !", input.name),
                "es" => format!("Hola, {}!", input.name),
                _ => format!("Hello, {}!", input.name),
            };

            let mut message = (0..input.repeat)
                .map(|_| greeting.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            if input.uppercase {
                message = message.to_uppercase();
            }

            ctx.shell("greet", ShellConfig::new(&format!("echo '{message}'")))
                .await?;

            Ok(())
        })
    }
}
