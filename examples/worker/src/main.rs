//! ironflow worker example.
//!
//! ```sh
//! cargo run -p ironflow-example-worker
//! ```
//!
//! Environment:
//! - `API_URL` (default: http://localhost:3000)
//! - `WORKER_TOKEN` (default: dev token)
//! - `CONCURRENCY` (default: 2)
//! - `POLL_INTERVAL_SECS` (default: 2)

use std::env;
use std::sync::Arc;
use std::time::Duration;

use tracing::info;
use tracing_subscriber::EnvFilter;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_worker::WorkerBuilder;

#[tokio::main]
async fn main() {
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

    let mut builder = WorkerBuilder::new(&api_url, &worker_token)
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .concurrency(concurrency)
        .poll_interval(Duration::from_secs(poll_interval));

    // Register all example workflows
    builder = builder
        .register(ironflow_workflows::WeatherReport)
        .register(ironflow_workflows::SystemAudit)
        .register(ironflow_workflows::GitInsight)
        .register(ironflow_workflows::Collect)
        .register(ironflow_workflows::Enrich)
        .register(ironflow_workflows::Report)
        .register(ironflow_workflows::CiPipeline);

    let worker = builder.build().expect("failed to build worker");

    info!("==============================================");
    info!("  ironflow worker");
    info!("  API: {api_url}");
    info!("  Concurrency: {concurrency}");
    info!("==============================================");

    if let Err(e) = worker.run().await {
        tracing::error!("worker error: {e}");
    }
}
