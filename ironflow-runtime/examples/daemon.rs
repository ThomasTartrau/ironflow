//! Minimal runtime daemon with a webhook.
//!
//! Demonstrates [`Runtime`] with a GitLab-authenticated webhook. The webhook
//! receives a JSON payload, extracts a description, and asks an agent to
//! summarise it.
//!
//! ```bash
//! # Set the GitLab webhook secret, then run:
//! export GITLAB_SECRET="my-webhook-secret"
//! cargo run --example daemon
//! ```

use ironflow_core::prelude::*;
use ironflow_runtime::prelude::*;
use serde_json::Value;

async fn on_webhook(payload: Value, provider: &ClaudeCodeProvider) {
    let description = payload["description"].as_str().unwrap_or("no description");

    let result = Agent::new()
        .system_prompt("You are a concise assistant. Respond in 2-3 lines maximum.")
        .prompt(&format!(
            "Summarise this webhook description:\n\n{description}"
        ))
        .model(Model::HAIKU)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(provider)
        .await;

    match result {
        Ok(agent) => {
            eprintln!("Webhook result: {}", agent.text());
            eprintln!("Cost: ${:.4}", agent.cost_usd().unwrap_or(0.0));
        }
        Err(e) => eprintln!("Webhook workflow failed: {e}"),
    }
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
    let gitlab_secret = std::env::var("GITLAB_SECRET")
        .expect("GITLAB_SECRET environment variable is required - set it to your webhook secret");

    Runtime::new()
        .webhook("/hook", WebhookAuth::gitlab(&gitlab_secret), {
            let p = provider.clone();
            move |payload| {
                let p = p.clone();
                async move { on_webhook(payload, &p).await }
            }
        })
        // Bind to localhost only; use a reverse proxy for public exposure.
        .serve("127.0.0.1:8080")
        .await?;

    Ok(())
}
