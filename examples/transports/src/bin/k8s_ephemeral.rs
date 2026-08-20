//! K8s ephemeral transport example.
//!
//! Creates a new pod for each invocation, reads logs, then deletes the pod.
//! Requires a reachable Kubernetes cluster (via kubeconfig or in-cluster).
//!
//! # Usage
//!
//! ```sh
//! K8S_IMAGE=ghcr.io/my-org/claude-runner:latest cargo run --bin k8s-ephemeral
//! ```

use std::env;

use ironflow_core::prelude::*;
use ironflow_core::providers::claude::K8sEphemeralProvider;

#[tokio::main]
async fn main() -> Result<(), OperationError> {
    let image = env::var("K8S_IMAGE").expect("K8S_IMAGE env var required");

    let provider = K8sEphemeralProvider::new(&image)
        .namespace("default")
        .service_account("claude-sa");

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
