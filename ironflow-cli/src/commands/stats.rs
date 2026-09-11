//! Stats commands: aggregate and historical.

use anyhow::Result;
use clap::{Args, Subcommand};
use ironflow_sdk::IronflowClient;

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
        /// Filter by workflow name.
        #[arg(long)]
        workflow: Option<String>,
        /// Time period: 24h, 7d, 30d, 90d. Defaults to 7d.
        #[arg(long, default_value = "7d")]
        period: String,
        /// Bucket granularity: 1h, 1d, 1w. Auto-derived from period when omitted.
        #[arg(long)]
        granularity: Option<String>,
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
        }) => {
            let response = client
                .stats_history(
                    workflow.as_deref(),
                    Some(period.as_str()),
                    granularity.as_deref(),
                )
                .await?;
            output::print_output(json_mode, &response, || {
                output::stats_history_table(&response.data)
            })?;
        }
    }
    Ok(())
}
