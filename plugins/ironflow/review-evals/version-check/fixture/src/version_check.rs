use ironflow_engine::config::{HttpConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Compares the deployed version with the latest published one.
pub struct VersionCheck;

impl WorkflowHandler for VersionCheck {
    fn name(&self) -> &str {
        "version-check"
    }

    fn description(&self) -> &str {
        "Compare the deployed version with the latest published release."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let latest = ctx
                .http(
                    "latest-version",
                    HttpConfig::get("https://releases.example.com/api/v1/latest"),
                )
                .await?;

            let version = latest.output["body"]["version"]
                .as_str()
                .unwrap_or_default()
                .to_string();

            ctx.shell(
                "compare",
                ShellConfig::exec("./scripts/compare-version.sh", &[version.as_str()]),
            )
            .await?;

            Ok(())
        })
    }
}
