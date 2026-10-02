//! Signal subcommands: send, list.

use std::fs::read_to_string;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use comfy_table::{ContentArrangement, Table};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ListSignalsFilter;
use ironflow_sdk::types::{SendSignalRequest, SignalDeliveryResponse, SignalResponse};
use serde_json::{Value, from_str, from_value, json};

use crate::output;

/// Arguments for the `signal` command group.
#[derive(Debug, Args)]
pub struct SignalArgs {
    /// Signal subcommand.
    #[command(subcommand)]
    pub command: SignalCommands,
}

/// Available signal subcommands.
#[derive(Debug, Subcommand)]
pub enum SignalCommands {
    /// Send a signal, resuming the runs waiting for it.
    Send {
        /// Signal name, e.g. `ci.pipeline_finished`.
        name: String,
        /// Occurrence key, e.g. a commit SHA.
        #[arg(long)]
        key: String,
        /// JSON payload, or `@path` to read it from a file. Defaults to `{}`.
        #[arg(long)]
        payload: Option<String>,
        /// Deduplication ID: sending it again delivers nothing.
        #[arg(long)]
        idempotency_id: Option<String>,
    },
    /// List received signals, newest first.
    List {
        /// Only signals with this exact name.
        #[arg(long)]
        name: Option<String>,
        /// Only signals with this exact key.
        #[arg(long)]
        key: Option<String>,
        /// Page number (1-based).
        #[arg(long)]
        page: Option<u32>,
        /// Items per page (max 100).
        #[arg(long)]
        per_page: Option<u32>,
    },
}

/// Read a payload file named by `--payload @path`.
fn read_payload_file(path: &str) -> Result<String> {
    read_to_string(path).with_context(|| format!("cannot read payload file '{path}'"))
}

/// Parse the `--payload` value: inline JSON, `@path` to a JSON file, or `{}`
/// when absent.
fn parse_payload(raw: Option<&str>) -> Result<Value> {
    let Some(raw) = raw else {
        return Ok(json!({}));
    };
    let text = match raw.strip_prefix('@') {
        Some(path) => read_payload_file(path)?,
        None => raw.to_string(),
    };
    let payload: Value = from_str(&text).context("--payload must be valid JSON")?;
    if !payload.is_object() {
        bail!("--payload must be a JSON object");
    }
    Ok(payload)
}

fn delivery_table(delivery: &SignalDeliveryResponse) -> Table {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec!["SIGNAL ID", "DUPLICATE", "RESUMED", "REJECTED"]);
    table.add_row(vec![
        delivery.signal_id.to_string(),
        delivery.duplicate.to_string(),
        delivery.resumed.len().to_string(),
        delivery.rejected.len().to_string(),
    ]);
    table
}

fn signals_table(items: &[SignalResponse]) -> Table {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec!["ID", "NAME", "KEY", "RECEIVED AT"]);
    for s in items {
        table.add_row(vec![
            s.id.to_string(),
            s.name.clone(),
            s.key.clone(),
            s.received_at.to_string(),
        ]);
    }
    table
}

/// Execute a signal subcommand.
///
/// # Errors
///
/// Returns an error on API failure or an invalid `--payload`.
pub async fn execute(client: &IronflowClient, args: &SignalArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        SignalCommands::Send {
            name,
            key,
            payload,
            idempotency_id,
        } => {
            let payload = parse_payload(payload.as_deref())?;
            let request: SendSignalRequest = from_value(json!({
                "name": name,
                "key": key,
                "payload": payload,
                "idempotency_id": idempotency_id,
            }))
            .context("cannot build the signal request")?;
            let response = client.send_signal(&request).await?;
            output::print_output(json_mode, &response, || delivery_table(&response.data))
        }
        SignalCommands::List {
            name,
            key,
            page,
            per_page,
        } => {
            let filter = ListSignalsFilter {
                name: name.clone(),
                key: key.clone(),
                page: *page,
                per_page: *per_page,
            };
            let response = client.list_signals(&filter).await?;
            output::print_output(json_mode, &response, || signals_table(&response.data))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::write;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn parse_payload_defaults_to_an_empty_object() {
        assert_eq!(parse_payload(None).unwrap(), json!({}));
    }

    #[test]
    fn parse_payload_accepts_inline_json() {
        let payload = parse_payload(Some(r#"{"status":"success"}"#)).unwrap();
        assert_eq!(payload, json!({"status": "success"}));
    }

    #[test]
    fn parse_payload_reads_a_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("payload.json");
        write(&path, r#"{"sha":"abc"}"#).unwrap();
        let payload = parse_payload(Some(&format!("@{}", path.display()))).unwrap();
        assert_eq!(payload, json!({"sha": "abc"}));
    }

    #[test]
    fn parse_payload_rejects_invalid_json_and_non_objects() {
        assert!(parse_payload(Some("{not json")).is_err());
        assert!(parse_payload(Some("[1, 2]")).is_err());
        assert!(parse_payload(Some("@/nonexistent/payload.json")).is_err());
    }
}
