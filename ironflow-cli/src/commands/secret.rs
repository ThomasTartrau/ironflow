//! Secret subcommands: rotate, key-status.

use anyhow::{Result, bail};
use clap::{Args, Subcommand, value_parser};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ApiResponse;
use ironflow_sdk::types::{KeyVersionsResponse, RotateSecretsRequest};
use serde::Serialize;
use uuid::Uuid;

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
/// Returns an error on API failure, or when a rotation left secrets behind
/// because they could not be decrypted.
pub async fn execute(client: &IronflowClient, args: &SecretArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        SecretCommands::Rotate(rotate_args) => rotate(client, rotate_args, json_mode).await,
        SecretCommands::KeyStatus => key_status(client, json_mode).await,
    }
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    /// Minimal parser harness: the real CLI wraps these in its own enum.
    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: SecretCommands,
    }

    #[test]
    fn rotate_defaults_to_active_version() {
        let cli = TestCli::try_parse_from(["test", "rotate"]).unwrap();
        let SecretCommands::Rotate(args) = cli.command else {
            panic!("expected rotate");
        };
        assert!(args.to_version.is_none());
        assert_eq!(args.batch_size, 100);
    }

    #[test]
    fn rotate_accepts_explicit_version_and_batch_size() {
        let cli =
            TestCli::try_parse_from(["test", "rotate", "--to-version", "2", "--batch-size", "50"])
                .unwrap();
        let SecretCommands::Rotate(args) = cli.command else {
            panic!("expected rotate");
        };
        assert_eq!(args.to_version, Some(2));
        assert_eq!(args.batch_size, 50);
    }

    #[test]
    fn rotate_rejects_non_positive_version() {
        assert!(TestCli::try_parse_from(["test", "rotate", "--to-version", "0"]).is_err());
        assert!(TestCli::try_parse_from(["test", "rotate", "--to-version", "-1"]).is_err());
    }

    #[test]
    fn rotate_rejects_out_of_range_batch_size() {
        assert!(TestCli::try_parse_from(["test", "rotate", "--batch-size", "0"]).is_err());
        assert!(TestCli::try_parse_from(["test", "rotate", "--batch-size", "1001"]).is_err());
    }

    #[test]
    fn key_status_takes_no_arguments() {
        let cli = TestCli::try_parse_from(["test", "key-status"]).unwrap();
        assert!(matches!(cli.command, SecretCommands::KeyStatus));
        assert!(TestCli::try_parse_from(["test", "key-status", "extra"]).is_err());
    }
}
