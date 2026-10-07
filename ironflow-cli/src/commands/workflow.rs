//! Workflow subcommands: list, get, pause, resume.

use anyhow::Result;
use clap::{Args, Subcommand};
use ironflow_sdk::IronflowClient;

use crate::output;

/// Arguments for the `workflow` command group.
#[derive(Debug, Args)]
pub struct WorkflowArgs {
    /// Workflow subcommand.
    #[command(subcommand)]
    pub command: WorkflowCommands,
}

/// Available workflow subcommands.
#[derive(Debug, Subcommand)]
pub enum WorkflowCommands {
    /// List all registered workflows.
    List,
    /// Get details of a specific workflow.
    Get {
        /// Workflow name.
        name: String,
    },
    /// Pause a workflow: workers stop picking its queued runs.
    Pause {
        /// Workflow name.
        name: String,
    },
    /// Resume a paused workflow: workers pick its queued runs again.
    Resume {
        /// Workflow name.
        name: String,
    },
}

/// Execute a workflow subcommand.
///
/// # Errors
///
/// Returns an error on API failure.
pub async fn execute(client: &IronflowClient, args: &WorkflowArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        WorkflowCommands::List => {
            let response = client.list_workflows().await?;
            output::print_output(json_mode, &response, || {
                output::workflows_table(&response.data)
            })?;
        }
        WorkflowCommands::Get { name } => {
            let response = client.get_workflow(name).await?;
            output::print_output(json_mode, &response, || {
                output::workflow_detail_table(&response.data)
            })?;
        }
        WorkflowCommands::Pause { name } => {
            let response = client.pause_workflow(name).await?;
            output::print_output(json_mode, &response, || {
                output::workflow_pause_table(&response.data)
            })?;
        }
        WorkflowCommands::Resume { name } => {
            let response = client.resume_workflow(name).await?;
            output::print_output(json_mode, &response, || {
                output::workflow_pause_table(&response.data)
            })?;
        }
    }
    Ok(())
}
