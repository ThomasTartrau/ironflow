//! Automated code review on the latest Git commit.
//!
//! Grabs the diff of the most recent commit, feeds it to an agent that acts as
//! a senior Rust reviewer, and prints the verdict.
//!
//! ```bash
//! cargo run --example code_review
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

    // Limit diff to 200 lines to keep context (and cost) reasonable.
    // Fall back gracefully when the repo has only one commit.
    let diff = Shell::new("git diff HEAD~1 2>/dev/null | head -200 || git show --stat HEAD")
        .timeout(Duration::from_secs(30))
        .await?;

    let review = Agent::new()
        .system_prompt(
            "You are a senior Rust engineer performing a code review. \
             Focus on correctness, safety, idiomatic patterns, and potential performance issues. \
             Be concise: flag real problems, skip nitpicks.",
        )
        .prompt(&format!(
            "Review the following commit diff and provide actionable feedback:\n\n```diff\n{}\n```",
            diff.stdout()
        ))
        .model(Model::Haiku)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&provider)
        .await?;

    eprintln!("--- Review completed ---");
    eprintln!(
        "cost=${:.4}  tokens={}/{}  duration={}ms",
        review.cost_usd().unwrap_or(0.0),
        review.input_tokens().unwrap_or(0),
        review.output_tokens().unwrap_or(0),
        review.duration_ms(),
    );
    println!("{}", review.text());

    Ok(())
}
