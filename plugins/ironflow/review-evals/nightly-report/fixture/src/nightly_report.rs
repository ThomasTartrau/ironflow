use std::fs::OpenOptions;
use std::io::Write;

use ironflow_engine::config::{ApprovalConfig, HttpConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use serde_json::json;

/// Collects the nightly stats, keeps a local history, publishes after review.
pub struct NightlyReport;

impl WorkflowHandler for NightlyReport {
    fn name(&self) -> &str {
        "nightly-report"
    }

    fn description(&self) -> &str {
        "Collect the nightly stats and publish them once a human has checked them."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let stats = ctx
                .shell("collect", ShellConfig::new("./scripts/collect-stats.sh"))
                .await?;

            let mut history = OpenOptions::new()
                .create(true)
                .append(true)
                .open("/var/lib/reports/nightly.csv")
                .map_err(|e| EngineError::StepConfig(e.to_string()))?;
            writeln!(history, "{},{}", ctx.run_id(), stats.stdout().trim())
                .map_err(|e| EngineError::StepConfig(e.to_string()))?;

            ctx.approval(
                "publish-report",
                ApprovalConfig::new("Publish tonight's report?"),
            )
            .await?;

            ctx.http(
                "publish",
                HttpConfig::post("https://reports.example.com/nightly")
                    .json(json!({ "stats": stats.stdout() })),
            )
            .await?;

            Ok(())
        })
    }
}
