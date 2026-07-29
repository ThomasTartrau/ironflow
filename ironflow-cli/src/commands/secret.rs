//! Secret subcommands: list, set, update, delete.
//!
//! No command in this module ever renders a secret value: the API's
//! `SecretResponse` does not carry one, and values are only ever sent.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::{SetSecretRequest, UpdateSecretRequest};
use std::slice;

use crate::confirm::{confirm, resolve_secret_value};
use crate::output;

/// Arguments for the `secret` command group.
#[derive(Debug, Args)]
pub struct SecretArgs {
    /// Secret subcommand.
    #[command(subcommand)]
    pub command: SecretCommands,
}

/// Available secret subcommands.
#[derive(Debug, Subcommand)]
pub enum SecretCommands {
    /// List secret keys. Values are never returned by the API.
    List,
    /// Create a secret, or replace the value of an existing one.
    Set {
        /// Secret key (namespaced, e.g. `workflows/inbox/gmail_token`).
        key: String,
        /// Secret value. Read from stdin when omitted, which keeps it out of
        /// the shell history and out of `ps` output.
        value: Option<String>,
    },
    /// Replace the value of an existing secret. Fails if the key is unknown.
    Update {
        /// Secret key.
        key: String,
        /// New secret value. Read from stdin when omitted.
        value: Option<String>,
    },
    /// Delete a secret.
    Delete {
        /// Secret key.
        key: String,
        /// Skip the interactive confirmation.
        #[arg(long)]
        yes: bool,
    },
}

/// Execute a secret subcommand.
///
/// # Errors
///
/// Returns an error on API failure, on an empty value, or when a destructive
/// command is not confirmed.
pub async fn execute(client: &IronflowClient, args: &SecretArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        SecretCommands::List => {
            let response = client.list_secrets().await?;
            output::print_output(json_mode, &response, || {
                output::secrets_table(&response.data)
            })?;
        }
        SecretCommands::Set { key, value } => {
            let value = resolve_secret_value(value.as_deref(), "secret value")?;
            let request: SetSecretRequest = SetSecretRequest::builder()
                .key(key.clone())
                .value(value)
                .try_into()
                .context("failed to build SetSecretRequest")?;

            let response = client.create_secret(&request).await?;
            output::print_output(json_mode, &response, || {
                output::secrets_table(slice::from_ref(&response.data))
            })?;
        }
        SecretCommands::Update { key, value } => {
            let value = resolve_secret_value(value.as_deref(), "secret value")?;
            let request: UpdateSecretRequest = UpdateSecretRequest::builder()
                .value(value)
                .try_into()
                .context("failed to build UpdateSecretRequest")?;

            let response = client.update_secret(key, &request).await?;
            output::print_output(json_mode, &response, || {
                output::secrets_table(slice::from_ref(&response.data))
            })?;
        }
        SecretCommands::Delete { key, yes } => {
            confirm(&format!("Delete secret '{key}'?"), *yes)?;
            client.delete_secret(key).await?;
            output::report_deletion(json_mode, "secret", key)?;
        }
    }
    Ok(())
}
