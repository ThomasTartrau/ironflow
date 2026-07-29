//! Audit log subcommands: list.

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ListAuditLogsFilter;
use ironflow_sdk::types::EventKind;
use uuid::Uuid;

use crate::commands::parse_enum;
use crate::output;

/// Arguments for the `audit-log` command group.
#[derive(Debug, Args)]
pub struct AuditLogArgs {
    /// Audit log subcommand.
    #[command(subcommand)]
    pub command: AuditLogCommands,
}

/// Available audit log subcommands.
#[derive(Debug, Subcommand)]
pub enum AuditLogCommands {
    /// List audit log entries with optional filters.
    List {
        /// Only entries attached to this run.
        #[arg(long = "run", value_name = "UUID")]
        run: Option<Uuid>,
        /// Only entries of this event type (e.g. `run_created`).
        #[arg(long = "type", value_name = "KIND", value_parser = parse_event_kind)]
        event_type: Option<EventKind>,
        /// Only entries recorded at or after this instant (RFC 3339).
        #[arg(long)]
        from: Option<DateTime<Utc>>,
        /// Only entries recorded at or before this instant (RFC 3339).
        #[arg(long)]
        to: Option<DateTime<Utc>>,
        /// Page number (1-based).
        #[arg(long)]
        page: Option<u32>,
        /// Items per page.
        #[arg(long)]
        per_page: Option<u32>,
    },
}

/// Every event kind the API records, in the order the enum declares them.
const ALL_EVENT_KINDS: [EventKind; 13] = [
    EventKind::RunCreated,
    EventKind::RunStatusChanged,
    EventKind::RunFailed,
    EventKind::RunBudgetExceeded,
    EventKind::StepCompleted,
    EventKind::StepFailed,
    EventKind::ApprovalRequested,
    EventKind::ApprovalGranted,
    EventKind::ApprovalRejected,
    EventKind::LogLine,
    EventKind::UserSignedIn,
    EventKind::UserSignedUp,
    EventKind::UserSignedOut,
];

/// Parse a `--type` value, listing the accepted values on failure.
///
/// # Errors
///
/// Returns the list of accepted event kinds when `raw` is not one of them.
fn parse_event_kind(raw: &str) -> Result<EventKind, String> {
    parse_enum(raw, &ALL_EVENT_KINDS, "event type")
}

/// Execute an audit log subcommand.
///
/// # Errors
///
/// Returns an error on API failure, including 403 for non-admin callers.
pub async fn execute(client: &IronflowClient, args: &AuditLogArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        AuditLogCommands::List {
            run,
            event_type,
            from,
            to,
            page,
            per_page,
        } => {
            let event_type = event_type.as_ref().map(ToString::to_string);
            let filter = ListAuditLogsFilter {
                run_id: *run,
                event_type: event_type.as_deref(),
                from: *from,
                to: *to,
                page: *page,
                per_page: *per_page,
            };

            let response = client.list_audit_logs_filtered(&filter).await?;
            output::print_output(json_mode, &response, || {
                output::audit_logs_table(&response.data)
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_event_kind_accepts_every_declared_kind() {
        for kind in ALL_EVENT_KINDS {
            let raw = kind.to_string();
            assert_eq!(parse_event_kind(&raw).unwrap(), kind);
        }
    }

    #[test]
    fn parse_event_kind_rejects_an_unknown_value_and_lists_the_valid_ones() {
        let err = parse_event_kind("run_exploded").unwrap_err();
        assert!(err.contains("unknown event type 'run_exploded'"), "{err}");
        assert!(err.contains("run_created"), "{err}");
    }
}
