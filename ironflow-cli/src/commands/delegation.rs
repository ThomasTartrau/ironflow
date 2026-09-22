//! Approval delegation subcommands: list, create, delete.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use comfy_table::{ContentArrangement, Table};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ListApprovalDelegationsFilter;
use ironflow_sdk::types::{ApprovalDelegationResponse, CreateApprovalDelegationRequest};
use uuid::Uuid;

use crate::confirm::confirm;
use crate::output;

/// Arguments for the `delegation` command group.
#[derive(Debug, Args)]
pub struct DelegationArgs {
    /// Delegation subcommand.
    #[command(subcommand)]
    pub command: DelegationCommands,
}

/// Available delegation subcommands.
#[derive(Debug, Subcommand)]
pub enum DelegationCommands {
    /// List the active approval delegations visible to you.
    List {
        /// Only delegations granted by this user (admin only).
        #[arg(long)]
        from_user: Option<Uuid>,
        /// Only delegations received by this user (admin only).
        #[arg(long)]
        to_user: Option<Uuid>,
        /// Page number (1-based).
        #[arg(long)]
        page: Option<u32>,
        /// Items per page (max 100).
        #[arg(long)]
        per_page: Option<u32>,
    },
    /// Delegate your approval power to another user.
    Create {
        /// User receiving the delegated approval power.
        to_user: Uuid,
        /// End of the window, as an RFC 3339 timestamp.
        #[arg(long)]
        until: String,
        /// Start of the window, as an RFC 3339 timestamp. Defaults to now.
        #[arg(long)]
        from: Option<String>,
        /// Glob restricting the delegation to matching workflow names.
        #[arg(long)]
        workflow: Option<String>,
    },
    /// Revoke an approval delegation.
    Delete {
        /// Delegation ID.
        id: Uuid,
        /// Skip the interactive confirmation.
        #[arg(long)]
        yes: bool,
    },
}

/// Parse an RFC 3339 timestamp, naming the flag it came from on failure.
fn parse_timestamp(raw: &str, flag: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|d| d.with_timezone(&Utc))
        .with_context(|| format!("--{flag} must be an RFC 3339 timestamp, got '{raw}'"))
}

fn delegations_table(items: &[ApprovalDelegationResponse]) -> Table {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        "ID",
        "FROM",
        "TO",
        "FROM DATE",
        "UNTIL",
        "WORKFLOW FILTER",
    ]);
    for d in items {
        let filter = d.workflow_filter.as_deref().unwrap_or("-");
        table.add_row(vec![
            d.id.to_string(),
            d.from_user_id.to_string(),
            d.to_user_id.to_string(),
            d.valid_from.to_string(),
            d.valid_until.to_string(),
            filter.to_string(),
        ]);
    }
    table
}

/// Execute a delegation subcommand.
///
/// # Errors
///
/// Returns an error on API failure, a malformed timestamp, or an unconfirmed
/// destructive command.
pub async fn execute(
    client: &IronflowClient,
    args: &DelegationArgs,
    json_mode: bool,
) -> Result<()> {
    match &args.command {
        DelegationCommands::List {
            from_user,
            to_user,
            page,
            per_page,
        } => {
            let filter = ListApprovalDelegationsFilter {
                from_user_id: *from_user,
                to_user_id: *to_user,
                page: *page,
                per_page: *per_page,
            };
            let response = client.list_approval_delegations_filtered(&filter).await?;
            output::print_output(json_mode, &response, || delegations_table(&response.data))
        }
        DelegationCommands::Create {
            to_user,
            until,
            from,
            workflow,
        } => {
            let valid_until = parse_timestamp(until, "until")?;
            let valid_from = from
                .as_deref()
                .map(|raw| parse_timestamp(raw, "from"))
                .transpose()?;
            let response = client
                .create_approval_delegation(&CreateApprovalDelegationRequest {
                    to_user_id: *to_user,
                    valid_from,
                    valid_until,
                    workflow_filter: workflow.clone(),
                })
                .await?;
            if json_mode {
                output::print_json(&response)?;
            } else {
                println!("Delegation {} created", response.data.id);
            }
            Ok(())
        }
        DelegationCommands::Delete { id, yes } => {
            let prompt = format!("Revoke delegation {id}?");
            confirm(&prompt, *yes)?;
            client.delete_approval_delegation(*id).await?;
            output::report_deletion(json_mode, "delegation", id.to_string())
        }
    }
}
