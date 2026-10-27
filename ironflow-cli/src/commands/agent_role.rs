//! Agent role subcommands: list, show, set-cap, clear-cap.

use std::slice::from_ref;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use comfy_table::{ContentArrangement, Table};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::{AgentRoleResponse, UpdateAgentRoleCostCapRequest};

use crate::output;

/// Arguments for the `agent-role` command group.
#[derive(Debug, Args)]
pub struct AgentRoleArgs {
    /// Agent role subcommand.
    #[command(subcommand)]
    pub command: AgentRoleCommands,
}

/// Available agent role subcommands.
#[derive(Debug, Subcommand)]
pub enum AgentRoleCommands {
    /// List agent roles with the cost spent, reserved and left this period.
    List {
        /// Page number (1-based).
        #[arg(long)]
        page: Option<u32>,
        /// Items per page (max 100).
        #[arg(long)]
        per_page: Option<u32>,
    },
    /// Show the cost and cap of one agent role.
    Show {
        /// Role name, as declared by `AgentRole::name`.
        name: String,
    },
    /// Set the operator cap of a role (admin only). It can only lower the
    /// cap declared in code.
    SetCap {
        /// Role name.
        name: String,
        /// Cap in USD over the role's period, at least 0 (0 disables the role).
        #[arg(long)]
        usd: f64,
    },
    /// Clear the operator cap of a role (admin only): the code cap applies.
    ClearCap {
        /// Role name.
        name: String,
    },
}

/// `$x.xxxx`, or `-` when there is no value.
fn usd(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("${v:.4}"))
}

fn roles_table(items: &[AgentRoleResponse]) -> Table {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        "NAME",
        "PERIOD",
        "SPENT",
        "RESERVED",
        "CAP",
        "REMAINING",
        "RESETS AT",
    ]);
    for role in items {
        table.add_row(vec![
            role.name.clone(),
            role.period.clone(),
            usd(Some(role.spent_usd)),
            usd(Some(role.reserved_usd)),
            usd(role.cap_usd),
            usd(role.remaining_usd),
            role.resets_at.to_string(),
        ]);
    }
    table
}

/// Build the cap update request, refusing a negative or non-finite cap.
fn cap_request(usd: Option<f64>) -> Result<UpdateAgentRoleCostCapRequest> {
    if let Some(cap) = usd
        && !(cap.is_finite() && cap >= 0.0)
    {
        bail!("--usd must be a number at least 0, got {cap}");
    }
    Ok(UpdateAgentRoleCostCapRequest {
        operator_cap_usd: usd,
    })
}

/// Execute an agent role subcommand.
///
/// # Errors
///
/// Returns an error on API failure or an invalid `--usd`.
pub async fn execute(client: &IronflowClient, args: &AgentRoleArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        AgentRoleCommands::List { page, per_page } => {
            let response = client.list_agent_roles(*page, *per_page).await?;
            output::print_output(json_mode, &response, || roles_table(&response.data))
        }
        AgentRoleCommands::Show { name } => {
            let response = client.get_agent_role(name).await?;
            output::print_output(json_mode, &response, || {
                roles_table(from_ref(&response.data))
            })
        }
        AgentRoleCommands::SetCap { name, usd } => {
            let request = cap_request(Some(*usd))?;
            let response = client.set_agent_role_cost_cap(name, &request).await?;
            output::print_output(json_mode, &response, || {
                roles_table(from_ref(&response.data))
            })
        }
        AgentRoleCommands::ClearCap { name } => {
            let request = cap_request(None)?;
            let response = client.set_agent_role_cost_cap(name, &request).await?;
            output::print_output(json_mode, &response, || {
                roles_table(from_ref(&response.data))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usd_formats_a_value_or_a_dash() {
        assert_eq!(usd(Some(1.5)), "$1.5000");
        assert_eq!(usd(None), "-");
    }

    #[test]
    fn cap_request_refuses_a_negative_or_nan_cap() {
        assert!(cap_request(Some(-1.0)).is_err());
        assert!(cap_request(Some(f64::NAN)).is_err());
        assert_eq!(cap_request(Some(2.5)).unwrap().operator_cap_usd, Some(2.5));
        assert_eq!(cap_request(None).unwrap().operator_cap_usd, None);
    }
}
