use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Input of the `checkout-branch` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckoutBranchInput {
    /// Branch to test, as pushed by the developer.
    pub branch: String,
}

/// Checks out a branch on the CI box and runs the test suite on it.
pub struct CheckoutBranch;

impl WorkflowHandler for CheckoutBranch {
    fn name(&self) -> &str {
        "checkout-branch"
    }

    fn description(&self) -> &str {
        "Check out a branch and run the test suite on it."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<CheckoutBranchInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: CheckoutBranchInput = ctx.input().await?;

            ctx.shell(
                "checkout",
                ShellConfig::new(&format!(
                    "git fetch origin && git checkout {}",
                    input.branch
                ))
                .dir("/srv/repo"),
            )
            .await?;

            ctx.shell(
                "test",
                ShellConfig::new("cargo test --workspace").dir("/srv/repo"),
            )
            .await?;

            Ok(())
        })
    }
}
