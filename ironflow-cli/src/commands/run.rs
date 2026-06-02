//! Run subcommands: create, list, get, cancel, approve, retry.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::slice;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ListRunsFilter;
use ironflow_sdk::types::CreateRunRequest;
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
    },
    /// List runs with optional filters.
    List {
        /// Filter by run status (pending, running, completed, failed, etc.).
        #[arg(long)]
        status: Option<String>,
        /// Filter by workflow name.
        #[arg(long)]
        workflow: Option<String>,
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
    /// Retry a failed run.
    Retry {
        /// Run UUID.
        id: Uuid,
    },
}

/// Resolve the payload from inline string or file.
fn resolve_payload(
    payload: Option<&str>,
    payload_file: Option<&PathBuf>,
) -> Result<serde_json::Value> {
    match (payload, payload_file) {
        (Some(raw), _) => serde_json::from_str(raw).context("invalid JSON in --payload"),
        (_, Some(path)) => {
            let content = fs::read_to_string(path)
                .with_context(|| format!("cannot read payload file: {}", path.display()))?;
            serde_json::from_str(&content)
                .with_context(|| format!("invalid JSON in {}", path.display()))
        }
        (None, None) => Ok(serde_json::Value::Object(serde_json::Map::new())),
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
        } => {
            let payload_value = resolve_payload(payload.as_deref(), payload_file.as_ref())?;
            let payload_map = payload_value
                .as_object()
                .context("payload must be a JSON object")?
                .clone();
            let request: CreateRunRequest = CreateRunRequest::builder()
                .workflow(workflow.clone())
                .payload(Some(payload_map))
                .try_into()
                .context("failed to build CreateRunRequest")?;

            let response = client.create_run(&request).await?;
            output::print_output(json_mode, &response, || {
                output::runs_table(slice::from_ref(&response.data))
            })?;
        }
        RunCommands::List {
            status,
            workflow,
            page,
            per_page,
        } => {
            let filter = ListRunsFilter {
                status: status.as_deref(),
                workflow: workflow.as_deref(),
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
                let mut out = std::io::stdout().lock();
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
        RunCommands::Retry { id } => {
            let response = client.retry_run(*id).await?;
            output::print_output(json_mode, &response, || {
                output::runs_table(slice::from_ref(&response.data))
            })?;
        }
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
