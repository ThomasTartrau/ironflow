use ironflow_engine::config::{ApprovalConfig, HttpConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use serde::{Deserialize, Serialize};
use serde_json::{from_str, json};

/// One vulnerable dependency reported by the audit script.
#[derive(Debug, Deserialize, Serialize)]
pub struct Finding {
    /// Crate name.
    pub package: String,
    /// Advisory id, such as `RUSTSEC-2024-0001`.
    pub advisory: String,
}

/// Report printed by `scripts/audit.sh`.
#[derive(Debug, Deserialize)]
pub struct Audit {
    /// Vulnerable dependencies, empty when the tree is clean.
    pub findings: Vec<Finding>,
}

/// Audits the dependencies and files a tracker issue when an operator confirms.
pub struct AuditReport;

impl WorkflowHandler for AuditReport {
    fn name(&self) -> &str {
        "audit-report"
    }

    fn description(&self) -> &str {
        "Audit the dependency tree and file an issue for the vulnerable crates."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let report = ctx
                .shell("audit", ShellConfig::new("./scripts/audit.sh --json"))
                .await?;
            let audit: Audit = from_str(report.stdout())?;

            ctx.http(
                "archive-report",
                HttpConfig::post("https://audit-archive.example.com/reports")
                    .json(json!({ "run": ctx.run_id(), "findings": audit.findings })),
            )
            .await?;

            if ctx.when_dynamic("findings present", !audit.findings.is_empty()) {
                ctx.approval(
                    "approve-issue",
                    ApprovalConfig::new(&format!(
                        "{} vulnerable crate(s). File an issue?",
                        audit.findings.len()
                    )),
                )
                .await?;

                ctx.http(
                    "file-issue",
                    HttpConfig::post("https://tracker.example.com/api/issues").json(
                        json!({ "title": "Vulnerable dependencies", "findings": audit.findings }),
                    ),
                )
                .await?;
            }

            Ok(())
        })
    }
}
