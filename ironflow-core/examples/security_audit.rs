//! Security audit with structured output.
//!
//! Runs `cargo audit --json` to detect known vulnerabilities, then asks an
//! agent to produce a typed risk assessment with severity ratings and
//! remediation steps.
//!
//! ```bash
//! cargo run --example security_audit
//! ```

use std::time::Duration;

use ironflow_core::prelude::*;

#[derive(Deserialize, JsonSchema, Debug)]
struct AuditReport {
    /// Total number of vulnerabilities found.
    total_vulnerabilities: u32,
    /// Highest severity across all findings: "none", "low", "medium", "high", or "critical".
    highest_severity: String,
    /// One entry per vulnerability.
    findings: Vec<Finding>,
    /// One-paragraph executive summary suitable for a Slack post.
    summary: String,
}

#[derive(Deserialize, JsonSchema, Debug)]
struct Finding {
    /// Crate name affected.
    crate_name: String,
    /// RUSTSEC advisory ID (e.g. "RUSTSEC-2024-0001").
    advisory_id: String,
    /// "low", "medium", "high", or "critical".
    severity: String,
    /// One-sentence description of the vulnerability.
    description: String,
    /// Recommended fix (e.g. "upgrade to >= 1.2.3").
    remediation: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let provider = ClaudeCodeProvider::new();
    let mut tracker = WorkflowTracker::new("security-audit");

    let audit = Shell::new("cargo audit --json 2>/dev/null || true")
        .timeout(Duration::from_secs(120))
        .await?;
    tracker.record_shell("cargo-audit", &audit);

    let agent = Agent::new()
        .system_prompt(
            "You are a security engineer. Analyze cargo audit JSON output and produce \
             a structured risk assessment. If there are no vulnerabilities, return an \
             empty findings array and set highest_severity to \"none\".",
        )
        .prompt(&format!(
            "Analyze this cargo audit report and produce a structured assessment:\n\n{}",
            audit.stdout()
        ))
        .model(Model::SONNET)
        .max_turns(1)
        .max_budget_usd(0.50)
        .output::<AuditReport>()
        .run(&provider)
        .await?;
    tracker.record_agent("risk-assessment", &agent);

    let report: AuditReport = agent.json()?;

    eprintln!("--- Security Audit Report ---");
    eprintln!("Vulnerabilities: {}", report.total_vulnerabilities);
    eprintln!("Highest severity: {}", report.highest_severity);
    for finding in &report.findings {
        eprintln!(
            "  [{severity}] {crate_name} ({id}): {desc} → {fix}",
            severity = finding.severity,
            crate_name = finding.crate_name,
            id = finding.advisory_id,
            desc = finding.description,
            fix = finding.remediation,
        );
    }
    eprintln!();
    println!("{}", report.summary);

    tracker.summary();

    Ok(())
}
