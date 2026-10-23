use ironflow_engine::config::{HttpConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Input of the `ping-hosts` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PingHostsInput {
    /// Hostnames to check.
    pub hosts: Vec<String>,
}

/// Pings every host and raises one alert listing the unreachable ones.
pub struct PingHosts;

impl WorkflowHandler for PingHosts {
    fn name(&self) -> &str {
        "ping-hosts"
    }

    fn description(&self) -> &str {
        "Ping a list of hosts and alert on the unreachable ones."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<PingHostsInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: PingHostsInput = ctx.input().await?;

            let mut unreachable = Vec::new();
            for host in &input.hosts {
                let ping = ctx
                    .shell(
                        "ping",
                        ShellConfig::exec("ping", &["-c", "1", host.as_str()])
                            .exit_code_as_output(),
                    )
                    .await?;
                if !ping.is_success() {
                    unreachable.push(host.clone());
                }
            }

            if !unreachable.is_empty() {
                ctx.http(
                    "alert",
                    HttpConfig::post("https://alerts.example.com/hosts")
                        .json(json!({ "unreachable": unreachable })),
                )
                .await?;
            }

            Ok(())
        })
    }
}
