//! Stats commands: aggregate and historical.

use anyhow::Result;
use clap::{Args, Subcommand};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::StatsHistoryParams;
use uuid::Uuid;

use crate::output;

/// Stats subcommand arguments.
///
/// # Examples
///
/// ```
/// use ironflow_cli::commands::stats::StatsArgs;
/// ```
#[derive(Debug, Args)]
pub struct StatsArgs {
    /// Stats subcommand. Omit for aggregate stats.
    #[command(subcommand)]
    pub command: Option<StatsCommands>,
}

/// Available stats subcommands.
///
/// # Examples
///
/// ```
/// use ironflow_cli::commands::stats::StatsCommands;
/// ```
#[derive(Debug, Subcommand)]
pub enum StatsCommands {
    /// Show time-bucketed historical statistics.
    History {
        /// Filter by workflow name (case-insensitive substring match).
        #[arg(long)]
        workflow: Option<String>,
        /// Time period: 24h, 7d, 30d, 90d. Defaults to 7d.
        #[arg(long, default_value = "7d")]
        period: String,
        /// Bucket granularity: 1h, 1d, 1w. Auto-derived from period when omitted.
        #[arg(long)]
        granularity: Option<String>,
        /// Filter by run status (pending, running, completed, failed, etc.).
        #[arg(long)]
        status: Option<String>,
        /// Filter by labels. Comma-separated `key:value` pairs.
        #[arg(long)]
        label: Option<String>,
        /// Filter by step presence (only applies to completed/cancelled runs).
        #[arg(long)]
        has_steps: Option<bool>,
        /// Filter by author: the user ID that triggered the run.
        ///
        /// Also matches runs triggered by one of that user's API keys.
        #[arg(long)]
        created_by: Option<Uuid>,
    },
}

/// Execute the `stats` command tree.
///
/// # Errors
///
/// Returns an error on API failure.
pub async fn execute(client: &IronflowClient, args: &StatsArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        None => {
            let response = client.get_stats().await?;
            output::print_output(json_mode, &response, || output::stats_table(&response.data))?;
        }
        Some(StatsCommands::History {
            workflow,
            period,
            granularity,
            status,
            label,
            has_steps,
            created_by,
        }) => {
            let params = StatsHistoryParams {
                workflow: workflow.as_deref(),
                period: Some(period.as_str()),
                granularity: granularity.as_deref(),
                status: status.as_deref(),
                label: label.as_deref(),
                has_steps: *has_steps,
                created_by: *created_by,
            };
            let response = client.stats_history_with(&params).await?;
            output::print_output(json_mode, &response, || {
                output::stats_history_table(&response.data)
            })?;
        }
    }
    Ok(())
}
