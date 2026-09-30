//! `ironflow accounts` -- manage Provider Accounts (admin only).
//!
//! The token is read from stdin only (`--token-stdin`), which keeps it out of
//! the shell history and out of `ps` output. No command prints it back.

use std::slice;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::{CreateProviderAccountRequest, UpdateProviderAccountRequest};

use crate::confirm::{confirm, resolve_secret_value};
use crate::output;

/// Arguments of `ironflow accounts`.
#[derive(Debug, Args)]
pub struct AccountArgs {
    /// Account subcommand.
    #[command(subcommand)]
    pub command: AccountCommands,
}

/// `ironflow accounts` subcommands.
#[derive(Debug, Subcommand)]
pub enum AccountCommands {
    /// Add an account. The token is checked against the provider first.
    Add {
        /// Unique name (lowercase slug, e.g. `perso-max`).
        name: String,
        /// Account kind.
        #[arg(long, default_value = "claude_subscription")]
        kind: String,
        /// Human-readable name.
        #[arg(long)]
        display_name: Option<String>,
        /// Read the token (e.g. from `claude setup-token`) from stdin.
        #[arg(long, required = true)]
        token_stdin: bool,
        /// Tag (repeatable).
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Priority, lower is preferred.
        #[arg(long)]
        priority: Option<i32>,
        /// Maximum concurrent steps.
        #[arg(long)]
        max_concurrency: Option<u32>,
        /// Utilization from which the account is shown as near its limit, in (0, 1].
        #[arg(long)]
        alert_threshold: Option<f64>,
        /// Subscription plan (`pro`, `max`).
        #[arg(long)]
        plan: Option<String>,
        /// When the token expires (RFC 3339).
        #[arg(long)]
        expires_at: Option<DateTime<Utc>>,
        /// Add the account disabled.
        #[arg(long)]
        disabled: bool,
    },
    /// List accounts with their current usage.
    List {
        /// Only accounts of this kind.
        #[arg(long)]
        kind: Option<String>,
    },
    /// Show one account and its windows.
    Show {
        /// Account name or UUID.
        account: String,
    },
    /// Show the current windows of an account.
    Usage {
        /// Account name or UUID.
        account: String,
    },
    /// Update an account. Omitted options are left unchanged.
    Update {
        /// Account name or UUID.
        account: String,
        /// New display name.
        #[arg(long)]
        display_name: Option<String>,
        /// Enable the account.
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        /// Disable the account.
        #[arg(long)]
        disable: bool,
        /// Tag (repeatable, replaces the list).
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// New priority.
        #[arg(long)]
        priority: Option<i32>,
        /// New maximum concurrent steps.
        #[arg(long, conflicts_with = "clear_max_concurrency")]
        max_concurrency: Option<u32>,
        /// Remove the concurrency limit.
        #[arg(long)]
        clear_max_concurrency: bool,
        /// New alert threshold, in (0, 1].
        #[arg(long)]
        alert_threshold: Option<f64>,
        /// New plan.
        #[arg(long)]
        plan: Option<String>,
        /// Replace the token, read from stdin.
        #[arg(long)]
        token_stdin: bool,
    },
    /// Delete an account and its token.
    Remove {
        /// Account name or UUID.
        account: String,
        /// Skip the interactive confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Check the stored token against the provider.
    Test {
        /// Account name or UUID.
        account: String,
    },
}

/// Execute an `ironflow accounts` subcommand.
///
/// # Errors
///
/// Returns an error when the API call fails, the token cannot be read, or
/// the user declines a confirmation.
pub async fn execute(client: &IronflowClient, args: &AccountArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        AccountCommands::Add {
            name,
            kind,
            display_name,
            token_stdin: _,
            tags,
            priority,
            max_concurrency,
            alert_threshold,
            plan,
            expires_at,
            disabled,
        } => {
            let token = resolve_secret_value(None, "token")?;
            let request: CreateProviderAccountRequest = CreateProviderAccountRequest::builder()
                .name(name.clone())
                .kind(kind.clone())
                .token(token)
                .display_name(display_name.clone())
                .enabled(Some(!disabled))
                .tags((!tags.is_empty()).then(|| tags.clone()))
                .priority(*priority)
                .max_concurrency(*max_concurrency)
                .alert_threshold(*alert_threshold)
                .plan(plan.clone())
                .expires_at(*expires_at)
                .try_into()
                .context("failed to build CreateProviderAccountRequest")?;
            let response = client.create_provider_account(&request).await?;
            output::print_output(json_mode, &response, || {
                output::provider_accounts_table(slice::from_ref(&response.data))
            })?;
        }
        AccountCommands::List { kind } => {
            let mut response = client.list_provider_accounts().await?;
            if let Some(kind) = kind {
                response.data.retain(|a| &a.kind == kind);
            }
            output::print_output(json_mode, &response, || {
                output::provider_accounts_table(&response.data)
            })?;
        }
        AccountCommands::Show { account } => {
            let response = client.get_provider_account(account).await?;
            output::print_output(json_mode, &response, || {
                output::provider_accounts_table(slice::from_ref(&response.data))
            })?;
            if !json_mode {
                println!(
                    "{}",
                    output::provider_account_windows_table(&response.data.windows)
                );
            }
        }
        AccountCommands::Usage { account } => {
            let response = client.provider_account_usage(account).await?;
            output::print_output(json_mode, &response, || {
                output::provider_account_windows_table(&response.data.windows)
            })?;
        }
        AccountCommands::Update {
            account,
            display_name,
            enable,
            disable,
            tags,
            priority,
            max_concurrency,
            clear_max_concurrency,
            alert_threshold,
            plan,
            token_stdin,
        } => {
            let token = if *token_stdin {
                Some(resolve_secret_value(None, "token")?)
            } else {
                None
            };
            let enabled = match (*enable, *disable) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            };
            let request: UpdateProviderAccountRequest = UpdateProviderAccountRequest::builder()
                .display_name(display_name.clone())
                .enabled(enabled)
                .tags((!tags.is_empty()).then(|| tags.clone()))
                .priority(*priority)
                .max_concurrency(*max_concurrency)
                .alert_threshold(*alert_threshold)
                .plan(plan.clone())
                .token(token)
                .try_into()
                .context("failed to build UpdateProviderAccountRequest")?;
            let mut response = client.update_provider_account(account, &request).await?;
            if *clear_max_concurrency {
                response = client
                    .clear_provider_account_max_concurrency(account)
                    .await?;
            }
            output::print_output(json_mode, &response, || {
                output::provider_accounts_table(slice::from_ref(&response.data))
            })?;
        }
        AccountCommands::Remove { account, yes } => {
            confirm(&format!("Delete provider account '{account}'?"), *yes)?;
            client.delete_provider_account(account).await?;
            output::report_deletion(json_mode, "provider account", account.clone())?;
        }
        AccountCommands::Test { account } => {
            let response = client.test_provider_account(account).await?;
            output::print_output(json_mode, &response, || {
                output::provider_account_windows_table(&response.data.windows)
            })?;
            if !json_mode {
                println!("result: {}", response.data.result);
            }
        }
    }
    Ok(())
}
