//! Worker: polls the API server for pending runs and executes the
//! workflow handlers. Start more workers to scale out.
//!
//! ```sh
//! cargo run -p worker
//! ```
//!
//! Configuration comes from the environment (see `.env.example`).

use std::env;
use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_worker::WorkerBuilder;

use workflows::handlers;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ironflow=debug".parse().expect("valid filter")),
        )
        .init();

    let api_url = env::var("API_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let worker_token =
        env::var("WORKER_TOKEN").unwrap_or_else(|_| "ironflow-dev-worker-token".to_string());
    let concurrency: usize = env::var("CONCURRENCY")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(2);
    let poll_interval: u64 = env::var("POLL_INTERVAL_SECS")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(2);

    // ---- Agent provider ---------------------------------------------------
    // ClaudeCodeProvider shells out to the `claude` CLI. See the setup
    // skill's `references/options.md` for HTTP providers (Anthropic API,
    // OpenAI, ...).
    let provider = Arc::new(ClaudeCodeProvider::new());

    let mut builder = WorkerBuilder::new(&api_url, &worker_token)
        .provider(provider)
        .concurrency(concurrency)
        .poll_interval(Duration::from_secs(poll_interval));

    // Same list as the server: one source of truth for both binaries.
    for handler in handlers() {
        builder = builder.register(handler);
    }

    let worker = builder.build().expect("failed to build worker");
    info!(api_url, concurrency, "ironflow worker started");

    if let Err(e) = worker.run().await {
        error!("worker error: {e}");
    }
}
