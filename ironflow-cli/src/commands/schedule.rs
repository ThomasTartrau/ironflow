//! Schedule subcommands: list, create, pause, resume, delete, trigger.

use anyhow::Result;
use clap::{Args, Subcommand};
use comfy_table::{ContentArrangement, Table};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::{CreateScheduleRequest, ScheduleResponse};
use uuid::Uuid;

use crate::confirm::confirm;
use crate::output;

/// Arguments for the `schedule` command group.
#[derive(Debug, Args)]
pub struct ScheduleArgs {
    /// Schedule subcommand.
    #[command(subcommand)]
    pub command: ScheduleCommands,
}

/// Available schedule subcommands.
#[derive(Debug, Subcommand)]
pub enum ScheduleCommands {
    /// List all schedules.
    List,
    /// Create a new schedule.
    Create {
        /// Workflow name.
        workflow: String,
        /// Cron expression (5 or 6 field format).
        cron: String,
        /// JSON inputs for the workflow (defaults to `{}`).
        #[arg(long, default_value = "{}")]
        inputs: String,
    },
    /// Pause a schedule (disable automatic triggers).
    Pause {
        /// Schedule ID.
        id: Uuid,
    },
    /// Resume a paused schedule.
    Resume {
        /// Schedule ID.
        id: Uuid,
    },
    /// Delete a schedule.
    Delete {
        /// Schedule ID.
        id: Uuid,
        /// Skip the interactive confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Trigger a schedule manually, creating a run immediately.
    Trigger {
        /// Schedule ID.
        id: Uuid,
    },
}

/// `active`, `paused` by a user, or `disabled: <reason>` when Ironflow
/// disabled the schedule on an error.
fn schedule_state(s: &ScheduleResponse) -> String {
    match (&s.disabled_at, &s.last_error) {
        (None, _) => "active".to_string(),
        (Some(_), Some(error)) => format!("disabled: {error}"),
        (Some(_), None) => "paused".to_string(),
    }
}

fn schedules_table(schedules: &[ScheduleResponse]) -> Table {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        "ID",
        "WORKFLOW",
        "CRON",
        "SOURCE",
        "ENABLED",
        "NEXT TRIGGER",
    ]);
    for s in schedules {
        let source = format!("{:?}", s.source).to_lowercase();
        table.add_row(vec![
            s.id.to_string(),
            s.workflow_name.clone(),
            s.cron_expression.clone(),
            source.to_string(),
            schedule_state(s),
            s.next_trigger_at
                .as_ref()
                .map(|d| d.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ]);
    }
    table
}

/// Execute a schedule subcommand.
///
/// # Errors
///
/// Returns an error on API failure, invalid JSON inputs, or an unconfirmed
/// destructive command.
pub async fn execute(client: &IronflowClient, args: &ScheduleArgs, json_mode: bool) -> Result<()> {
    match &args.command {
        ScheduleCommands::List => {
            let response = client.list_schedules().await?;
            if json_mode {
                output::print_json(&response)?;
            } else {
                println!("{}", schedules_table(&response.data));
            }
            Ok(())
        }
        ScheduleCommands::Create {
            workflow,
            cron,
            inputs,
        } => {
            let parsed_inputs: serde_json::Value =
                serde_json::from_str(inputs).map_err(|e| anyhow::anyhow!("invalid JSON: {e}"))?;
            let response = client
                .create_schedule(&CreateScheduleRequest {
                    workflow_name: workflow.clone(),
                    cron_expression: cron.clone(),
                    inputs: Some(parsed_inputs),
                })
                .await?;
            if json_mode {
                output::print_json(&response)?;
            } else {
                println!("Schedule {} created", response.data.id);
            }
            Ok(())
        }
        ScheduleCommands::Pause { id } => {
            let response = client.pause_schedule(*id).await?;
            if json_mode {
                output::print_json(&response)?;
            } else {
                println!("Schedule {} paused", response.data.id);
            }
            Ok(())
        }
        ScheduleCommands::Resume { id } => {
            let response = client.resume_schedule(*id).await?;
            if json_mode {
                output::print_json(&response)?;
            } else {
                println!("Schedule {} resumed", response.data.id);
            }
            Ok(())
        }
        ScheduleCommands::Delete { id, yes } => {
            let prompt = format!("Delete schedule {id}?");
            confirm(&prompt, *yes)?;
            client.delete_schedule(*id).await?;
            if json_mode {
                output::print_json(&serde_json::json!({"deleted": id.to_string()}))?;
            } else {
                println!("Schedule {id} deleted");
            }
            Ok(())
        }
        ScheduleCommands::Trigger { id } => {
            let response = client.trigger_schedule(*id).await?;
            if json_mode {
                output::print_json(&response)?;
            } else {
                println!("Schedule {} triggered", response.data.id);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json};

    use super::*;

    fn schedule(disabled_at: Option<&str>, last_error: Option<&str>) -> ScheduleResponse {
        from_value(json!({
            "id": "01a10fdb-f467-7982-826b-0c99470c7264",
            "workflow_name": "deploy",
            "cron_expression": "0 0 30 2 *",
            "inputs": {},
            "source": "api",
            "disabled_at": disabled_at,
            "last_triggered_at": null,
            "next_trigger_at": null,
            "last_error": last_error,
            "created_by_user_id": null,
            "created_at": "2026-10-06T08:00:00Z",
            "updated_at": "2026-10-06T08:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn schedule_state_shows_why_ironflow_disabled_a_schedule() {
        let s = schedule(
            Some("2026-10-06T08:00:00Z"),
            Some("cannot compute next trigger"),
        );
        assert_eq!(schedule_state(&s), "disabled: cannot compute next trigger");
        assert!(
            schedules_table(&[s])
                .to_string()
                .contains("disabled: cannot compute next trigger")
        );
    }

    #[test]
    fn schedule_state_tells_a_user_pause_from_an_active_schedule() {
        assert_eq!(
            schedule_state(&schedule(Some("2026-10-06T08:00:00Z"), None)),
            "paused"
        );
        assert_eq!(schedule_state(&schedule(None, None)), "active");
    }
}
