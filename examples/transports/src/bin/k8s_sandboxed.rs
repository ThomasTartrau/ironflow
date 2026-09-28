//! Sandboxed K8s ephemeral transport example.
//!
//! Runs Claude Code in a hardened pod: non-root, read-only root filesystem,
//! all capabilities dropped, OAuth token read from a Kubernetes Secret, a
//! locked-down managed-settings preset, and the `anthropic-only` egress
//! profile label. A background task reaps orphaned pods every five minutes.
//!
//! Apply the manifests of `examples/k8s/sandbox/` first. Requires a reachable
//! Kubernetes cluster (via kubeconfig or in-cluster).
//!
//! # Usage
//!
//! ```sh
//! K8S_IMAGE=registry.gitlab.com/thomastartrau/ironflow/ironflow-claude-runner:2.1.274-1 \
//!     cargo run --bin k8s-sandboxed
//! ```

use std::env;
use std::time::Duration;

use ironflow_core::prelude::*;
use ironflow_core::providers::claude::K8sEphemeralProvider;

#[tokio::main]
async fn main() -> Result<(), OperationError> {
    let image = env::var("K8S_IMAGE").expect("K8S_IMAGE env var required");

    let provider = K8sEphemeralProvider::sandboxed(&image)
        .namespace("ironflow-agents")
        .oauth_token_from_secret("claude-oauth", "token")
        .managed_settings_preset("locked", "claude-managed-locked")
        .default_managed_settings("locked")
        .egress_profile("anthropic-only")
        .timeout(Duration::from_secs(600));
    let _reaper = provider.spawn_orphan_reaper(Duration::from_secs(300));

    // Outside the engine, `run_scope` sets the run/step labels the engine
    // stamps on every agent step.
    let config = AgentConfig::new("List the top-level directories of /data/repos.")
        .max_budget_usd(0.10)
        .read_only_pvc("repos", "/data/repos")
        .run_scope("demo-run", "investigate");

    let result = Agent::from_config(config).run(&provider).await?;

    println!("Response: {}", result.text());
    println!("Model: {}", result.model().unwrap_or("unknown"));
    println!("Duration: {}ms", result.duration_ms());

    Ok(())
}
