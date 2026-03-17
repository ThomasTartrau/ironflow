//! Multi-step incident report generation.
//!
//! Collects recent Git activity, current service health, and dependency status,
//! then feeds everything to an agent that produces a structured incident report.
//! Demonstrates multi-step workflows with [`WorkflowTracker`] cost aggregation.
//!
//! ```bash
//! cargo run --example incident_report
//! ```

use std::time::Duration;

use ironflow_core::prelude::*;

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
    let mut tracker = WorkflowTracker::new("incident-report");

    // Steps 1-3: collect data in parallel (all independent)
    let (git_log, build_status, deps) = tokio::try_join!(
        Shell::new("git log --since='24 hours ago' --oneline --stat")
            .timeout(Duration::from_secs(10))
            .run(),
        Shell::new("cargo check --message-format=short 2>&1 || true")
            .timeout(Duration::from_secs(120))
            .run(),
        Shell::new("cargo tree --depth 1 2>/dev/null || true")
            .timeout(Duration::from_secs(30))
            .run(),
    )?;
    tracker.record_shell("git-log", &git_log);
    tracker.record_shell("cargo-check", &build_status);
    tracker.record_shell("dependency-tree", &deps);

    // Step 4: agent synthesizes an incident report
    let report = Agent::new()
        .system_prompt(
            "You are an SRE writing an incident report for a Rust project. \
             Synthesize the provided data into a clear, actionable report. \
             Structure your response with: Summary, Recent Changes, Build Status, \
             Dependencies of Note, and Recommended Actions.",
        )
        .prompt(&format!(
            "Generate an incident report from the following data:\n\n\
             ## Recent commits (last 24h)\n```\n{git_log}\n```\n\n\
             ## Build status\n```\n{build_status}\n```\n\n\
             ## Dependency tree\n```\n{deps}\n```",
            git_log = git_log.stdout(),
            build_status = build_status.stdout(),
            deps = deps.stdout(),
        ))
        .model(Model::Sonnet)
        .max_turns(1)
        .max_budget_usd(0.50)
        .run(&provider)
        .await?;
    tracker.record_agent("synthesize-report", &report);

    println!("{}", report.text());

    eprintln!("\n--- Workflow metrics ---");
    tracker.summary();

    Ok(())
}
