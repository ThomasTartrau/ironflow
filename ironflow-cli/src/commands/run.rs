//! Run subcommands: create, list, get, cancel, approve, retry, watch, diff.

use std::fs;
use std::io::{Write as _, stdout};
use std::path::PathBuf;
use std::slice;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use futures_util::StreamExt;
use humantime::format_duration;
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ListRunsFilter;
use ironflow_sdk::types::{CreateRunRequest, RunStatus};
use serde_json::{Map, Value, from_str, json, to_string};
use tokio::time::timeout as tokio_timeout;
use uuid::Uuid;

use crate::output;

/// Arguments for the `run` command group.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Run subcommand.
    #[command(subcommand)]
    pub command: RunCommands,
}

/// Available run subcommands.
#[derive(Debug, Subcommand)]
pub enum RunCommands {
    /// Create a new run for a workflow.
    Create {
        /// Workflow name to trigger.
        workflow: String,
        /// JSON payload (inline string).
        #[arg(long, group = "payload_source")]
        payload: Option<String>,
        /// Path to a JSON file containing the payload.
        #[arg(long, group = "payload_source")]
        payload_file: Option<PathBuf>,
        /// How many times to replay the run automatically after a transient
        /// failure. Defaults to 0 (no automatic retry).
        #[arg(long)]
        max_retries: Option<u32>,
        /// Idempotency key making the call safe to replay.
        ///
        /// Reusing the same key returns the run it already created instead of
        /// starting a second one. Valid for 24 hours. At most 255 printable
        /// ASCII characters.
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Maximum cumulative cost for this run, in USD. Overrides the
        /// workflow and server defaults.
        #[arg(long = "max-cost", value_name = "USD")]
        max_cost: Option<f64>,
    },
    /// List runs with optional filters.
    List {
        /// Filter by run status (pending, running, completed, failed, etc.).
        #[arg(long)]
        status: Option<String>,
        /// Filter by workflow name.
        #[arg(long)]
        workflow: Option<String>,
        /// Filter by author: the user ID that triggered the run.
        ///
        /// Also matches runs triggered by one of that user's API keys.
        #[arg(long)]
        created_by: Option<Uuid>,
        /// Page number (1-based).
        #[arg(long)]
        page: Option<u32>,
        /// Items per page.
        #[arg(long)]
        per_page: Option<u32>,
    },
    /// Get details of a specific run.
    Get {
        /// Run UUID.
        id: Uuid,
    },
    /// Cancel a pending or running run.
    Cancel {
        /// Run UUID.
        id: Uuid,
    },
    /// Approve a run waiting for approval.
    Approve {
        /// Run UUID.
        id: Uuid,
    },
    /// Reject a run waiting for approval, failing it.
    Reject {
        /// Run UUID.
        id: Uuid,
    },
    /// Retry a failed run.
    Retry {
        /// Run UUID.
        id: Uuid,
        /// Force retry even when the handler version has changed since the
        /// original run.
        #[arg(long)]
        force: bool,
    },
    /// Watch a run in real time via SSE.
    Watch {
        /// Run UUID.
        id: Uuid,
        /// Only show step transitions, not log data.
        #[arg(long)]
        no_logs: bool,
        /// Stop watching after this duration (e.g. "30s", "5m", "1h").
        #[arg(long, value_parser = parse_humantime)]
        timeout: Option<Duration>,
    },
    /// Compare two runs of the same workflow side by side.
    Diff {
        /// First run UUID.
        run_a: Uuid,
        /// Second run UUID.
        run_b: Uuid,
    },
}

/// Parse a human-readable duration string (e.g. "30s", "5m", "1h").
fn parse_humantime(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s).map_err(|e| e.to_string())
}

/// Terminal event types that signal the run is done.
const TERMINAL_EVENTS: &[&str] = &["run_completed", "run_failed", "run_cancelled"];

/// Resolve the payload from inline string or file.
fn resolve_payload(payload: Option<&str>, payload_file: Option<&PathBuf>) -> Result<Value> {
    match (payload, payload_file) {
        (Some(raw), _) => from_str(raw).context("invalid JSON in --payload"),
        (_, Some(path)) => {
            let content = fs::read_to_string(path)
                .with_context(|| format!("cannot read payload file: {}", path.display()))?;
            from_str(&content).with_context(|| format!("invalid JSON in {}", path.display()))
        }
        (None, None) => Ok(Value::Object(Map::new())),
    }
}

/// Reject a `--max-cost` value the API would refuse anyway.
///
/// Catching it client-side turns a 400 round-trip into an immediate, readable
/// error.
///
/// # Errors
///
/// Returns an error when the value is negative or not a finite number.
fn validate_max_cost(max_cost: Option<f64>) -> Result<()> {
    match max_cost {
        Some(value) if !value.is_finite() => {
            anyhow::bail!("--max-cost must be a finite number, got {value}")
        }
        Some(value) if value < 0.0 => {
            anyhow::bail!("--max-cost must be zero or positive, got {value}")
        }
        _ => Ok(()),
    }
}

/// Execute a run subcommand.
///
/// # Errors
///
/// Returns an error on API failure or invalid input.
pub async fn execute(
    client: &IronflowClient,
    args: &RunArgs,
    json_mode: bool,
    _verbose: bool,
) -> Result<()> {
    match &args.command {
        RunCommands::Create {
            workflow,
            payload,
            payload_file,
            max_retries,
            idempotency_key,
            max_cost,
        } => {
            validate_max_cost(*max_cost)?;
            let payload_value = resolve_payload(payload.as_deref(), payload_file.as_ref())?;
            let payload_map = payload_value
                .as_object()
                .context("payload must be a JSON object")?
                .clone();
            let request: CreateRunRequest = CreateRunRequest::builder()
                .workflow(workflow.clone())
                .payload(Some(payload_map))
                // The generated SDK models the field as i32; the API rejects
                // anything negative, and clap already refuses it here.
                .max_retries(max_retries.map(|n| n as i32))
                .max_cost_usd(*max_cost)
                .try_into()
                .context("failed to build CreateRunRequest")?;

            let response = match idempotency_key {
                Some(key) => client.create_run_idempotent(&request, key).await?,
                None => client.create_run(&request).await?,
            };
            output::print_output(json_mode, &response, || {
                output::runs_table(slice::from_ref(&response.data))
            })?;
        }
        RunCommands::List {
            status,
            workflow,
            created_by,
            page,
            per_page,
        } => {
            let filter = ListRunsFilter {
                status: status.as_deref(),
                workflow: workflow.as_deref(),
                created_by: *created_by,
                page: *page,
                per_page: *per_page,
                ..Default::default()
            };
            let response = client.list_runs_filtered(&filter).await?;
            output::print_output(json_mode, &response, || output::runs_table(&response.data))?;
        }
        RunCommands::Get { id } => {
            let response = client.get_run(*id).await?;
            output::print_output(json_mode, &response, || {
                output::run_detail_table(&response.data)
            })?;

            if !json_mode && !response.data.steps.is_empty() {
                let mut out = stdout().lock();
                writeln!(out)?;
                writeln!(out, "Steps:")?;
                writeln!(out, "{}", output::steps_table(&response.data.steps))?;
            }
        }
        RunCommands::Cancel { id } => {
            let response = client.cancel_run(*id).await?;
            output::print_output(json_mode, &response, || {
                output::runs_table(slice::from_ref(&response.data))
            })?;
        }
        RunCommands::Approve { id } => {
            let response = client.approve_run(*id).await?;
            output::print_output(json_mode, &response, || {
                output::runs_table(slice::from_ref(&response.data))
            })?;
        }
        RunCommands::Reject { id } => {
            let response = client.reject_run(*id).await?;
            output::print_output(json_mode, &response, || {
                output::runs_table(slice::from_ref(&response.data))
            })?;
        }
        RunCommands::Retry { id, force } => {
            let response = client.retry_run(*id, *force).await?;
            output::print_output(json_mode, &response, || {
                output::runs_table(slice::from_ref(&response.data))
            })?;
        }
        RunCommands::Watch {
            id,
            no_logs,
            timeout,
        } => {
            execute_watch(client, *id, *no_logs, *timeout, json_mode).await?;
        }
        RunCommands::Diff { run_a, run_b } => {
            execute_diff(client, *run_a, *run_b, json_mode).await?;
        }
    }
    Ok(())
}

/// Watch a run in real time via SSE.
async fn execute_watch(
    client: &IronflowClient,
    run_id: Uuid,
    no_logs: bool,
    timeout: Option<Duration>,
    json_mode: bool,
) -> Result<()> {
    let run = client.get_run(run_id).await?;
    let status = run.data.run.status;
    if matches!(
        status,
        RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
    ) {
        if json_mode {
            output::print_output(json_mode, &run, || output::run_detail_table(&run.data))?;
        } else {
            let mut out = stdout().lock();
            writeln!(out, "Run {run_id} already in terminal state: {status}")?;
        }
        return Ok(());
    }

    let watch_fut = async {
        let mut stream = client.events(Some(run_id), None).await?;
        let mut out = stdout().lock();

        while let Some(event) = stream.next().await {
            match event {
                Ok(ev) => {
                    if no_logs
                        && !ev.event_type.starts_with("run_")
                        && !ev.event_type.starts_with("step_")
                    {
                        continue;
                    }

                    if json_mode {
                        let obj = json!({
                            "event": ev.event_type,
                            "data": ev.data,
                        });
                        writeln!(out, "{}", to_string(&obj)?)?;
                    } else {
                        writeln!(out, "[{}] {}", ev.event_type, ev.data)?;
                    }

                    if TERMINAL_EVENTS.contains(&ev.event_type.as_str()) {
                        break;
                    }
                }
                Err(e) => {
                    return Err(anyhow!("SSE stream error: {e}"));
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    };

    match timeout {
        Some(dur) => {
            tokio_timeout(dur, watch_fut).await.unwrap_or_else(|_| {
                eprintln!("Timeout reached after {}", format_duration(dur));
                Ok(())
            })?;
        }
        None => {
            watch_fut.await?;
        }
    }

    Ok(())
}

/// Compare two runs of the same workflow side by side.
async fn execute_diff(
    client: &IronflowClient,
    run_a_id: Uuid,
    run_b_id: Uuid,
    json_mode: bool,
) -> Result<()> {
    if run_a_id == run_b_id {
        bail!("both run IDs are the same; nothing to diff");
    }

    let (a, b) = tokio::try_join!(client.get_run(run_a_id), client.get_run(run_b_id))?;

    if a.data.run.workflow_name != b.data.run.workflow_name {
        bail!(
            "cannot diff runs from different workflows: '{}' vs '{}'",
            a.data.run.workflow_name,
            b.data.run.workflow_name
        );
    }

    if json_mode {
        let diff = json!({
            "run_a": a.data,
            "run_b": b.data,
        });
        output::print_json(&diff)?;
    } else {
        let table = output::run_diff_table(&a.data, &b.data);
        let mut out = stdout().lock();
        writeln!(out, "{table}")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn resolve_payload_none_returns_empty_object() {
        let value = resolve_payload(None, None).unwrap();
        assert!(value.is_object());
        assert!(value.as_object().unwrap().is_empty());
    }

    #[test]
    fn resolve_payload_inline_valid_json() {
        let value = resolve_payload(Some(r#"{"key": "value"}"#), None).unwrap();
        assert_eq!(value["key"], "value");
    }

    #[test]
    fn resolve_payload_inline_invalid_json() {
        let result = resolve_payload(Some("not json"), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid JSON"));
    }

    #[test]
    fn resolve_payload_file_valid() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, r#"{{"workflow": "test"}}"#).unwrap();
        let path = tmp.path().to_path_buf();

        let value = resolve_payload(None, Some(&path)).unwrap();
        assert_eq!(value["workflow"], "test");
    }

    #[test]
    fn resolve_payload_file_not_found() {
        let path = PathBuf::from("/nonexistent/payload.json");
        let result = resolve_payload(None, Some(&path));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot read"));
    }

    #[test]
    fn resolve_payload_file_invalid_json() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "not valid json").unwrap();
        let path = tmp.path().to_path_buf();

        let result = resolve_payload(None, Some(&path));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid JSON"));
    }
}
