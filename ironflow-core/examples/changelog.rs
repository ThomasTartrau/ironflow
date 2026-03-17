//! Changelog generation with record/replay.
//!
//! Reads the Git log between the last two tags (or the last 20 commits) and
//! asks an agent to produce a user-facing changelog grouped by category.
//!
//! Uses [`RecordReplayProvider`] so the first run captures the real agent
//! response, and subsequent runs replay it without spending tokens.
//!
//! ```bash
//! # First run - records the fixture
//! IRONFLOW_RECORD=1 cargo run --example changelog
//!
//! # Subsequent runs - replays from fixture (zero cost)
//! cargo run --example changelog
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

    let claude = ClaudeCodeProvider::new();
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let provider = RecordReplayProvider::new(claude, fixtures);

    // Try to find the range between the two most recent tags; fall back to HEAD~20
    let tag_range = Shell::new(
        "tags=$(git tag --sort=-creatordate | head -2) && \
         count=$(echo \"$tags\" | wc -l | tr -d ' ') && \
         if [ \"$count\" -ge 2 ]; then \
           older=$(echo \"$tags\" | tail -1) && \
           newer=$(echo \"$tags\" | head -1) && \
           echo \"${older}..${newer}\"; \
         else \
           echo 'HEAD~20..HEAD'; \
         fi",
    )
    .timeout(Duration::from_secs(10))
    .await?;
    let range = tag_range.stdout().trim();

    // Pass the range via an environment variable to avoid shell injection from
    // malicious tag names (e.g. a tag containing `; rm -rf /`).
    let git_log = Shell::new(
        "git log $RANGE --pretty=format:'%h %s (%an)' --no-merges 2>/dev/null || \
         git log --oneline -20 --no-merges",
    )
    .env("RANGE", range)
    .timeout(Duration::from_secs(30))
    .await?;

    let changelog = Agent::new()
        .system_prompt(
            "You are a technical writer. Given a list of Git commits, produce a \
             user-facing changelog in Markdown. Group entries under: Added, Changed, \
             Fixed, Removed, Security. Omit empty groups. Rewrite commit messages \
             into clear, user-oriented descriptions.",
        )
        .prompt(&format!(
            "Generate a changelog from these commits ({range}):\n\n```\n{}\n```",
            git_log.stdout()
        ))
        .model(Model::Haiku)
        .max_turns(1)
        .max_budget_usd(0.50)
        .run(&provider)
        .await?;

    eprintln!("--- Changelog generated ---");
    eprintln!(
        "cost=${:.4}  tokens={}/{}",
        changelog.cost_usd().unwrap_or(0.0),
        changelog.input_tokens().unwrap_or(0),
        changelog.output_tokens().unwrap_or(0),
    );
    println!("{}", changelog.text());

    Ok(())
}
