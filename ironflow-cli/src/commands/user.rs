//! User subcommands: list, create, delete, set-role.

use std::slice;

use anyhow::{Context, Result};
use clap::{ArgGroup, Args, Subcommand};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::{CreateUserRequest, UpdateRoleRequest};
use uuid::Uuid;

use crate::confirm::{confirm, resolve_secret_value};
use crate::output;

/// Arguments for the `user` command group.
#[derive(Debug, Args)]
pub struct UserArgs {
    /// User subcommand.
    #[command(subcommand)]
    pub command: UserCommands,
}

/// Available user subcommands.
#[derive(Debug, Subcommand)]
pub enum UserCommands {
    /// List users.
    List,
    /// Create a user.
    Create {
        /// Display username.
        username: String,
        /// Email address.
        #[arg(long)]
        email: String,
        /// Plaintext password (min 8 characters). Read from stdin when
        /// omitted, which keeps it out of the shell history.
        #[arg(long)]
        password: Option<String>,
        /// Grant admin rights to the new user.
        #[arg(long)]
        admin: bool,
    },
    /// Delete a user.
    Delete {
        /// User UUID.
        id: Uuid,
        /// Skip the interactive confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Promote a user to admin or demote them to member.
    #[command(group(ArgGroup::new("role").required(true).args(["admin", "member"])))]
    SetRole {
        /// User UUID.
        id: Uuid,
        /// Grant admin rights.
        #[arg(long)]
        admin: bool,
        /// Revoke admin rights.
        #[arg(long)]
        member: bool,
    },
}

/// Execute a user subcommand.
///
/// # Errors
///
/// Returns an error on API failure, on an empty password, or when a
/// destructive command is not confirmed.
pub async fn execute(client: &IronflowClient, args: &UserArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        UserCommands::List => {
            let response = client.list_users().await?;
            output::print_output(json_mode, &response, || output::users_table(&response.data))?;
        }
        UserCommands::Create {
            username,
            email,
            password,
            admin,
        } => {
            let password = resolve_secret_value(password.as_deref(), "password")?;
            let request: CreateUserRequest = CreateUserRequest::builder()
                .username(username.clone())
                .email(email.clone())
                .password(password)
                .is_admin(*admin)
                .try_into()
                .context("failed to build CreateUserRequest")?;

            let response = client.create_user(&request).await?;
            output::print_output(json_mode, &response, || {
                output::users_table(slice::from_ref(&response.data))
            })?;
        }
        UserCommands::Delete { id, yes } => {
            confirm(&format!("Delete user '{id}'?"), *yes)?;
            client.delete_user(*id).await?;
            output::report_deletion(json_mode, "user", id.to_string())?;
        }
        // `--admin` and `--member` are an exclusive, required clap group, so
        // `admin` alone carries the whole decision.
        UserCommands::SetRole { id, admin, .. } => {
            let request: UpdateRoleRequest = UpdateRoleRequest::builder()
                .is_admin(*admin)
                .try_into()
                .context("failed to build UpdateRoleRequest")?;

            let response = client.update_role(*id, &request).await?;
            output::print_output(json_mode, &response, || {
                output::users_table(slice::from_ref(&response.data))
            })?;
        }
    }
    Ok(())
}
