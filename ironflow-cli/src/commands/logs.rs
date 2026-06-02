//! Log streaming via SSE.

use std::io::Write as _;

use anyhow::Result;
use clap::Args;
use futures_util::StreamExt;
use ironflow_sdk::IronflowClient;
use uuid::Uuid;

/// Arguments for the `logs` command.
#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Run UUID to stream logs for.
    pub run_id: Uuid,
    /// Keep streaming until the run reaches a terminal state.
    #[arg(long)]
    pub follow: bool,
}

/// Terminal event types that signal the run is done.
const TERMINAL_EVENTS: &[&str] = &["run_completed", "run_failed", "run_cancelled"];

/// Execute the `logs` command.
///
/// # Errors
///
/// Returns an error on SSE connection failure.
pub async fn execute(client: &IronflowClient, args: &LogsArgs, json_mode: bool) -> Result<()> {
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

                if !args.follow {
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
