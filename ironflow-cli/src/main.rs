//! # ironflow-cli
//!
//! Command-line interface for the [Ironflow](https://gitlab.com/ThomasTartrau/ironflow)
//! workflow engine. Consumes [`ironflow_sdk`] to manage runs, workflows,
//! stream logs, and view statistics.
//!
//! ## Configuration
//!
//! The CLI resolves configuration in this priority order (highest wins):
//!
//! 1. CLI flags (`--url`, `--api-key`)
//! 2. Environment variables (`IRONFLOW_URL`, `IRONFLOW_API_KEY`)
//! 3. TOML file at `~/.ironflow.toml`

mod commands;
mod config;
mod output;

use anyhow::Result;
use clap::{Parser, Subcommand};
use ironflow_sdk::IronflowClient;
use tracing_subscriber::EnvFilter;

use crate::commands::logs::LogsArgs;
use crate::commands::run::RunArgs;
use crate::commands::workflow::WorkflowArgs;

/// CLI for the Ironflow workflow engine.
#[derive(Debug, Parser)]
#[command(
    name = "ironflow-cli",
    version,
    about = "Drive the Ironflow workflow engine from the terminal"
)]
struct Cli {
    /// Output raw JSON instead of formatted tables.
    #[arg(long, global = true)]
    json: bool,

    /// Show verbose output (e.g. full step details in `run get`).
    #[arg(long, global = true)]
    verbose: bool,

    /// Override the Ironflow API base URL.
    #[arg(long, global = true, env = "IRONFLOW_URL")]
    url: Option<String>,

    /// Override the API key for authentication.
    #[arg(long, global = true, env = "IRONFLOW_API_KEY")]
    api_key: Option<String>,

    /// Command to execute.
    #[command(subcommand)]
    command: Commands,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
enum Commands {
    /// Manage workflow runs.
    Run(RunArgs),
    /// Manage workflows.
    Workflow(WorkflowArgs),
    /// Stream run logs via SSE.
    Logs(LogsArgs),
    /// Show global statistics.
    Stats,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = config::load(cli.url.as_deref(), cli.api_key.as_deref())?;
    let client = IronflowClient::new(&config.base_url, &config.api_key);

    match &cli.command {
        Commands::Run(args) => commands::run::execute(&client, args, cli.json, cli.verbose).await,
        Commands::Workflow(args) => commands::workflow::execute(&client, args, cli.json).await,
        Commands::Logs(args) => commands::logs::execute(&client, args, cli.json).await,
        Commands::Stats => commands::stats::execute(&client, cli.json).await,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parse_run_list() {
        let cli = Cli::try_parse_from(["ironflow-cli", "run", "list"]).unwrap();
        assert!(!cli.json);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_list_with_json() {
        let cli = Cli::try_parse_from(["ironflow-cli", "--json", "run", "list"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn parse_run_create_with_payload() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "create",
            "deploy",
            "--payload",
            r#"{"env": "prod"}"#,
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_create_with_payload_file() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "create",
            "deploy",
            "--payload-file",
            "/tmp/payload.json",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_create_payload_and_file_conflict() {
        let result = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "create",
            "deploy",
            "--payload",
            "{}",
            "--payload-file",
            "/tmp/p.json",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_run_get() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "get",
            "01234567-89ab-cdef-0123-456789abcdef",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_cancel() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "cancel",
            "01234567-89ab-cdef-0123-456789abcdef",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_approve() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "approve",
            "01234567-89ab-cdef-0123-456789abcdef",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_retry() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "retry",
            "01234567-89ab-cdef-0123-456789abcdef",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_list_with_filters() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "list",
            "--status",
            "completed",
            "--workflow",
            "deploy",
            "--page",
            "2",
            "--per-page",
            "50",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_workflow_list() {
        let cli = Cli::try_parse_from(["ironflow-cli", "workflow", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Workflow(_)));
    }

    #[test]
    fn parse_workflow_get() {
        let cli = Cli::try_parse_from(["ironflow-cli", "workflow", "get", "deploy"]).unwrap();
        assert!(matches!(cli.command, Commands::Workflow(_)));
    }

    #[test]
    fn parse_logs() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "logs",
            "01234567-89ab-cdef-0123-456789abcdef",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Logs(_)));
    }

    #[test]
    fn parse_logs_follow() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "logs",
            "01234567-89ab-cdef-0123-456789abcdef",
            "--follow",
        ])
        .unwrap();
        if let Commands::Logs(args) = &cli.command {
            assert!(args.follow);
        } else {
            panic!("expected Logs command");
        }
    }

    #[test]
    fn parse_stats() {
        let cli = Cli::try_parse_from(["ironflow-cli", "stats"]).unwrap();
        assert!(matches!(cli.command, Commands::Stats));
    }

    #[test]
    fn parse_verbose_flag() {
        let cli = Cli::try_parse_from(["ironflow-cli", "--verbose", "stats"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn parse_url_override() {
        let cli = Cli::try_parse_from([
            "ironflow-cli",
            "--url",
            "https://custom.example.com",
            "stats",
        ])
        .unwrap();
        assert_eq!(cli.url.as_deref(), Some("https://custom.example.com"));
    }

    #[test]
    fn parse_invalid_uuid_rejected() {
        let result = Cli::try_parse_from(["ironflow-cli", "run", "get", "not-a-uuid"]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_no_command_fails() {
        let result = Cli::try_parse_from(["ironflow-cli"]);
        assert!(result.is_err());
    }
}
