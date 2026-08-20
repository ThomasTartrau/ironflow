//! SSH transport example.
//!
//! Connects to a remote host via SSH and runs `claude` there.
//! Requires the `claude` binary to be installed on the remote host.
//!
//! # Usage
//!
//! ```sh
//! SSH_HOST=build-server SSH_USER=deploy SSH_PASSWORD=secret cargo run --bin ssh-transport
//! ```

use std::env;

use ironflow_core::prelude::*;
use ironflow_core::providers::claude::SshProvider;
use ironflow_core::providers::claude::ssh::HostKeyPolicy;

#[tokio::main]
async fn main() -> Result<(), OperationError> {
    let host = env::var("SSH_HOST").expect("SSH_HOST env var required");
    let user = env::var("SSH_USER").expect("SSH_USER env var required");
    let password = env::var("SSH_PASSWORD").expect("SSH_PASSWORD env var required");

    // For production, use HostKeyPolicy::Fingerprint or HostKeyPolicy::KnownHostsFile
    let provider = SshProvider::new(&host, &user)
        .password(&password)
        .host_key_policy(HostKeyPolicy::AcceptAll);

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
