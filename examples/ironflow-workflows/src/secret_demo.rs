use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};
use std::collections::HashMap;

pub struct SecretDemo;

impl WorkflowHandler for SecretDemo {
    fn name(&self) -> &str {
        "secret-demo"
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Demonstrates reading a secret from the encrypted store \
                          and using it in a workflow step. Expects a secret with \
                          key 'demo/api-key' to be configured via the dashboard."
                .to_string(),
            source_code: Some(include_str!("secret_demo.rs").to_string()),
            sub_workflows: Vec::new(),
            category: Some("examples".to_string()),
            version: self.version().map(str::to_string),
            input_schema: None,
            default_labels: HashMap::new(),
            schedule: None,
            default_max_cost_usd: None,
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let store = ctx.store();

            let secret = store
                .get_secret("demo/api-key")
                .await
                .map_err(EngineError::Store)?
                .ok_or_else(|| {
                    EngineError::StepConfig(
                        "secret 'demo/api-key' not found, create it in Settings > Secrets"
                            .to_string(),
                    )
                })?;

            let value_len = secret.value.len();
            ctx.shell(
                "use-secret",
                ShellConfig::new(&format!(
                    "echo 'Secret successfully retrieved: value_length={value_len} chars'"
                )),
            )
            .await?;

            Ok(())
        })
    }
}
