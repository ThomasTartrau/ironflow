use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Input of the `ping-fleet` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PingFleetInput {
    /// Hostnames of the fleet.
    pub hosts: Vec<String>,
}

/// Pings the fleet and drains the unreachable hosts once an operator agrees.
pub struct PingFleet;

impl WorkflowHandler for PingFleet {
    fn name(&self) -> &str {
        "ping-fleet"
    }

    fn description(&self) -> &str {
        "Ping the fleet and drain the unreachable hosts after approval."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<PingFleetInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: PingFleetInput = ctx.input().await?;

            let mut unreachable = Vec::new();
            for (i, host) in input.hosts.iter().enumerate() {
                let ping = ctx
                    .shell(
                        &format!("ping-{i}"),
                        ShellConfig::exec("ping", &["-c", "1", host.as_str()])
                            .exit_code_as_output(),
                    )
                    .await?;
                if !ping.is_success() {
                    unreachable.push(host.clone());
                }
            }

            if unreachable.is_empty() {
                return Ok(());
            }

            let message = format!(
                "{} host(s) unreachable: {}. Drain them?",
                unreachable.len(),
                unreachable.join(", ")
            );
            ctx.approval("approve-drain", ApprovalConfig::new(&message))
                .await?;

            for (i, host) in unreachable.iter().enumerate() {
                ctx.shell(
                    &format!("drain-{i}"),
                    ShellConfig::exec("./scripts/drain.sh", &[host.as_str()]),
                )
                .await?;
            }

            Ok(())
        })
    }
}
