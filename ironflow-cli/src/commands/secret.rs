//! Secret subcommands: list, set, update, delete, rotate, key-status.
//!
//! No command in this module ever renders a secret value: the API's
//! `SecretResponse` does not carry one, and values are only ever sent.

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, value_parser};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ApiResponse;
use ironflow_sdk::types::{
    KeyVersionsResponse, RotateSecretsRequest, SetSecretRequest, UpdateSecretRequest,
};
use serde::Serialize;
use std::slice;
use uuid::Uuid;

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
    /// Re-encrypt every stored secret with a key version.
    ///
    /// Runs batch by batch until the whole stock is on the target version.
    /// Safe to interrupt and rerun: secrets already rotated are skipped.
    Rotate(RotateArgs),
    /// Show which encryption key versions are configured and in use.
    KeyStatus,
}

/// Arguments for `secret rotate`.
#[derive(Debug, Args)]
pub struct RotateArgs {
    /// Target key version. Defaults to the server's active version.
    #[arg(long, value_parser = value_parser!(i32).range(1..))]
    pub to_version: Option<i32>,

    /// Secrets to re-encrypt per request.
    #[arg(long, default_value_t = 100, value_parser = value_parser!(i32).range(1..=1000))]
    pub batch_size: i32,
}

/// Summary of a completed rotation, for `--json` output.
#[derive(Debug, Serialize)]
struct RotationSummary {
    /// Key version the stock was rotated to.
    to_version: i32,
    /// Total secrets re-encrypted.
    rotated: u64,
    /// Total secrets skipped because they could not be decrypted.
    failed: u64,
    /// Number of requests issued.
    batches: u64,
}

/// Execute a secret subcommand.
///
/// # Errors
///
/// Returns an error on API failure, on an empty value, when a destructive
/// command is not confirmed, or when a rotation left secrets behind because
/// they could not be decrypted.
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
        SecretCommands::Rotate(rotate_args) => rotate(client, rotate_args, json_mode).await?,
        SecretCommands::KeyStatus => key_status(client, json_mode).await?,
    }
    Ok(())
}

/// Drive a rotation to completion, one batch per request.
async fn rotate(client: &IronflowClient, args: &RotateArgs, json_mode: bool) -> Result<()> {
    let mut cursor: Option<Uuid> = None;
    let mut to_version = args.to_version;
    let mut rotated = 0u64;
    let mut failed = 0u64;
    let mut batches = 0u64;

    loop {
        let request = RotateSecretsRequest {
            to_version,
            batch_size: Some(args.batch_size),
            after_id: cursor,
        };

        let response = client.rotate_secrets(&request).await?;
        let batch = response.data;
        batches += 1;
        rotated += batch.rotated as u64;
        failed += batch.failed as u64;

        // The server resolves an omitted version to its active one; pin it
        // for the remaining batches so a mid-rotation config change cannot
        // send the tail of the stock to a different version.
        to_version = Some(batch.to_version);

        if !json_mode {
            let done = rotated + failed;
            println!(
                "rotating to version {}: {done} done, {} remaining",
                batch.to_version, batch.remaining
            );
        }

        // A batch with no cursor means nothing was left to read.
        let Some(last_id) = batch.last_id else { break };
        if batch.remaining == 0 {
            break;
        }
        cursor = Some(last_id);
    }

    let summary = RotationSummary {
        to_version: to_version.unwrap_or_default(),
        rotated,
        failed,
        batches,
    };

    if json_mode {
        output::print_json(&summary)?;
    } else {
        println!(
            "done: {rotated} secret(s) rotated to version {} in {batches} batch(es)",
            summary.to_version
        );
    }

    if failed > 0 {
        bail!(
            "{failed} secret(s) could not be decrypted and were left on their previous key version; check the server logs for the affected keys"
        );
    }

    Ok(())
}

/// Report the key ring status.
async fn key_status(client: &IronflowClient, json_mode: bool) -> Result<()> {
    let response = client.secret_key_versions().await?;
    output::print_output(json_mode, &response, || {
        output::key_versions_table(&response.data)
    })?;

    warn_on_missing(&response);

    Ok(())
}

/// Print a warning when stored secrets reference an unconfigured key version.
fn warn_on_missing(response: &ApiResponse<KeyVersionsResponse>) {
    if response.data.missing.is_empty() {
        return;
    }

    let missing = response
        .data
        .missing
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "warning: key version(s) {missing} are used by stored secrets but not configured; those secrets cannot be read and the server will refuse to restart"
    );
}
