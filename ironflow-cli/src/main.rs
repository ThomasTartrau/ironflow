//! # ironflow-cli
//!
//! Command-line interface for the [Ironflow](https://gitlab.com/ThomasTartrau/ironflow)
//! workflow engine. Consumes [`ironflow_sdk`] to manage runs, workflows,
//! secrets, API keys, users and audit logs, stream logs, and view statistics.
//!
//! ## Configuration
//!
//! The CLI resolves configuration in this priority order (highest wins):
//!
//! 1. CLI flags (`--url`, `--api-key`)
//! 2. Environment variables (`IRONFLOW_URL`, `IRONFLOW_API_KEY`)
//! 3. TOML file at `~/.ironflow.toml`

use anyhow::Result;
use clap::Parser;
use ironflow_cli::cli::{Cli, dispatch};
use ironflow_cli::config;
use ironflow_sdk::IronflowClient;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = config::load(cli.url.as_deref(), cli.api_key.as_deref())?;
    let client = IronflowClient::new(&config.base_url, &config.api_key);

    dispatch(&client, &cli).await
}
