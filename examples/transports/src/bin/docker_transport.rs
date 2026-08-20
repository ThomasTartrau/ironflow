//! Docker transport example.
//!
//! Executes `claude` inside a running Docker container via `docker exec`.
//! The container must already be running and have the `claude` binary installed.
//!
//! # Usage
//!
//! ```sh
//! DOCKER_CONTAINER=claude-worker cargo run --bin docker-transport
//! ```

use std::env;

use ironflow_core::prelude::*;
use ironflow_core::providers::claude::DockerProvider;

#[tokio::main]
async fn main() -> Result<(), OperationError> {
    let container = env::var("DOCKER_CONTAINER").expect("DOCKER_CONTAINER env var required");

    let provider = DockerProvider::new(&container).working_dir("/workspace");

    let result = Agent::new()
        .prompt("What is 2 + 2?")
        .max_budget_usd(0.10)
        .run(&provider)
        .await?;

    println!("Response: {}", result.text());
    println!("Model: {}", result.model().unwrap_or("unknown"));
    println!("Duration: {}ms", result.duration_ms());

    Ok(())
}
