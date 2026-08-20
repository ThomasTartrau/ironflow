//! K8s persistent transport example.
//!
//! Reuses a long-running worker pod and executes commands via `kubectl exec`.
//! If the pod does not exist, it is created automatically.
//! Requires a reachable Kubernetes cluster (via kubeconfig or in-cluster).
//!
//! # Usage
//!
//! ```sh
//! K8S_IMAGE=ghcr.io/my-org/claude-runner:latest cargo run --bin k8s-persistent
//! ```

use std::env;

use ironflow_core::prelude::*;
use ironflow_core::providers::claude::K8sPersistentProvider;

#[tokio::main]
async fn main() -> Result<(), OperationError> {
    let image = env::var("K8S_IMAGE").expect("K8S_IMAGE env var required");

    let provider = K8sPersistentProvider::new(&image)
        .pod_name("claude-worker")
        .namespace("default");

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
