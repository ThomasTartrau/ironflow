//! Log retrieval and streaming via SSE.

use std::io::Write as _;

use anyhow::Result;
use clap::Args;
use futures_util::StreamExt;
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ListRunLogsFilter;
use uuid::Uuid;

/// Arguments for the `logs` command.
#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Run UUID to retrieve logs for.
    pub run_id: Uuid,
    /// Keep streaming until the run reaches a terminal state.
    #[arg(long)]
    pub follow: bool,
    /// Filter by step ID.
    #[arg(long)]
    pub step_id: Option<Uuid>,
    /// Filter by output stream (`stdout`, `stderr`, `system`).
    #[arg(long)]
    pub stream: Option<String>,
    /// Maximum number of entries to return per page (only without --follow).
    #[arg(long)]
    pub limit: Option<u32>,
}

/// Terminal event types that signal the run is done.
const TERMINAL_EVENTS: &[&str] = &["run_completed", "run_failed", "run_cancelled"];

/// Execute the `logs` command.
///
/// Without `--follow`, retrieves persisted logs via `GET /api/v1/runs/:id/logs`.
/// With `--follow`, streams live logs via SSE until the run reaches a terminal state.
///
/// # Errors
///
/// Returns an error on API or SSE connection failure.
pub async fn execute(client: &IronflowClient, args: &LogsArgs, json_mode: bool) -> Result<()> {
    if args.follow {
        return execute_follow(client, args, json_mode).await;
    }

    let mut out = std::io::stdout().lock();
    let mut cursor: Option<Uuid> = None;

    loop {
        let filter = ListRunLogsFilter {
            step_id: args.step_id,
            stream: args.stream.as_deref(),
            cursor,
            limit: args.limit,
        };

        let response = client.get_run_logs(args.run_id, &filter).await?;

        for entry in &response.data {
            if json_mode {
                writeln!(out, "{}", serde_json::to_string(&entry)?)?;
            } else {
                writeln!(
                    out,
                    "[{}] [{}] {}",
                    entry.step_name, entry.stream, entry.line
                )?;
            }
        }

        let has_more = response
            .meta
            .as_ref()
            .and_then(|m| m.extra.get("has_more"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !has_more {
            break;
        }

        cursor = response
            .meta
            .as_ref()
            .and_then(|m| m.extra.get("next_cursor"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());
    }

    Ok(())
}

/// Stream logs via SSE (--follow mode).
async fn execute_follow(client: &IronflowClient, args: &LogsArgs, json_mode: bool) -> Result<()> {
    let mut stream = client.events(Some(args.run_id), None).await?;
    let mut out = std::io::stdout().lock();

    while let Some(event) = stream.next().await {
        match event {
            Ok(ev) => {
                if json_mode {
                    let obj = serde_json::json!({
                        "event": ev.event_type,
                        "data": ev.data,
                    });
                    writeln!(out, "{}", serde_json::to_string(&obj)?)?;
                } else {
                    writeln!(out, "[{}] {}", ev.event_type, ev.data)?;
                }

                if TERMINAL_EVENTS.contains(&ev.event_type.as_str()) {
                    break;
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("SSE stream error: {e}"));
            }
        }
    }

    Ok(())
}
