//! API key subcommands: list, create, scopes, delete.

use std::io::Write as _;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::{ApiKeyScope, CreateApiKeyRequest};
use uuid::Uuid;

use crate::commands::parse_enum;
use crate::confirm::confirm;
use crate::output;

/// Arguments for the `api-key` command group.
#[derive(Debug, Args)]
pub struct ApiKeyArgs {
    /// API key subcommand.
    #[command(subcommand)]
    pub command: ApiKeyCommands,
}

/// Available API key subcommands.
#[derive(Debug, Subcommand)]
pub enum ApiKeyCommands {
    /// List API keys. The raw key is never listed, only its prefix.
    List,
    /// Create an API key. The raw key is printed once and never again.
    Create {
        /// Human-readable name for the key.
        name: String,
        /// Scope to grant. Repeat for several scopes. Run `api-key scopes`
        /// to list the accepted values.
        #[arg(long = "scope", value_name = "SCOPE", required = true, value_parser = parse_scope)]
        scopes: Vec<ApiKeyScope>,
        /// Expiration date (RFC 3339, e.g. `2026-12-31T23:59:59Z`).
        #[arg(long)]
        expires_at: Option<DateTime<Utc>>,
        /// Per-key rate limit override (requests per minute). 0 disables
        /// rate limiting for this key.
        #[arg(long)]
        rate_limit_override: Option<u32>,
    },
    /// List the scopes an API key can be granted.
    Scopes,
    /// Delete an API key.
    Delete {
        /// API key UUID.
        id: Uuid,
        /// Skip the interactive confirmation.
        #[arg(long)]
        yes: bool,
    },
}

/// Every scope the API accepts, in the order the enum declares them.
const ALL_SCOPES: [ApiKeyScope; 6] = [
    ApiKeyScope::WorkflowsRead,
    ApiKeyScope::RunsRead,
    ApiKeyScope::RunsWrite,
    ApiKeyScope::RunsManage,
    ApiKeyScope::StatsRead,
    ApiKeyScope::Admin,
];

/// Parse a `--scope` value, listing the accepted values on failure.
///
/// # Errors
///
/// Returns the list of accepted scopes when `raw` is not one of them.
fn parse_scope(raw: &str) -> Result<ApiKeyScope, String> {
    parse_enum(raw, &ALL_SCOPES, "scope")
}

/// Execute an API key subcommand.
///
/// # Errors
///
/// Returns an error on API failure or when a destructive command is not
/// confirmed.
pub async fn execute(client: &IronflowClient, args: &ApiKeyArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        ApiKeyCommands::List => {
            let response = client.list_api_keys().await?;
            output::print_output(json_mode, &response, || {
                output::api_keys_table(&response.data)
            })?;
        }
        ApiKeyCommands::Create {
            name,
            scopes,
            expires_at,
            rate_limit_override,
        } => {
            let mut builder = CreateApiKeyRequest::builder()
                .name(name.clone())
                .scopes(scopes.clone())
                .expires_at(*expires_at);
            if let Some(val) = rate_limit_override {
                builder = builder.rate_limit_override(*val as i32);
            }
            let request: CreateApiKeyRequest = builder
                .try_into()
                .context("failed to build CreateApiKeyRequest")?;

            let response = client.create_api_key(&request).await?;

            if !json_mode {
                let mut stderr = std::io::stderr();
                writeln!(stderr, "This is the only time the key is shown.")?;
            }

            output::print_output(json_mode, &response, || {
                output::created_api_key_table(&response.data)
            })?;
        }
        ApiKeyCommands::Scopes => {
            let response = client.available_scopes().await?;
            output::print_output(json_mode, &response, || {
                output::scopes_table(&response.data)
            })?;
        }
        ApiKeyCommands::Delete { id, yes } => {
            confirm(&format!("Delete API key '{id}'?"), *yes)?;
            client.delete_api_key(*id).await?;
            output::report_deletion(json_mode, "api-key", id.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scope_accepts_every_declared_scope() {
        for scope in ALL_SCOPES {
            let raw = scope.to_string();
            assert_eq!(parse_scope(&raw).unwrap(), scope);
        }
    }

    #[test]
    fn parse_scope_rejects_an_unknown_value_and_lists_the_valid_ones() {
        let err = parse_scope("root").unwrap_err();
        assert!(err.contains("unknown scope 'root'"), "{err}");
        assert!(err.contains("runs_read"), "{err}");
        assert!(err.contains("admin"), "{err}");
    }

    #[test]
    fn parse_scope_is_case_sensitive() {
        assert!(parse_scope("ADMIN").is_err());
    }
}
