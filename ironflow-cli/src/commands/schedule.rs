//! Schedule subcommands: list, create, pause, resume, delete, trigger.

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum, value_parser};
use comfy_table::{ContentArrangement, Table};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::{CatchupPolicy, CreateScheduleRequest, OverlapPolicy, ScheduleResponse};
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
        /// Queue priority, from -100 to 100, given to every run the schedule
        /// creates. Defaults to the workflow priority.
        #[arg(
            long,
            allow_negative_numbers = true,
            value_parser = value_parser!(i16).range(-100..=100)
        )]
        priority: Option<i16>,
        /// What the schedule does with the occurrences it missed while no
        /// server fired it. Defaults to `latest`.
        #[arg(long, value_enum)]
        catchup: Option<CatchupArg>,
        /// Most runs created to catch up under `--catchup all`, from 1 to
        /// 1000. Defaults to 10.
        #[arg(long, value_parser = value_parser!(i32).range(1..=1000))]
        catchup_max: Option<i32>,
        /// How far back, in seconds, a missed occurrence is still caught up,
        /// from 60 to 2592000 (30 days). Defaults to 86400 (one day).
        #[arg(
            long = "catchup-window",
            value_name = "SECONDS",
            value_parser = value_parser!(i32).range(60..=2_592_000)
        )]
        catchup_window_secs: Option<i32>,
        /// What the schedule does when an occurrence comes while one of its
        /// runs is still active. Defaults to `allow`.
        #[arg(long, value_enum)]
        overlap: Option<OverlapArg>,
        /// IANA timezone the cron expression is evaluated in, e.g.
        /// `Europe/Paris`. Defaults to `UTC`.
        #[arg(long, value_name = "IANA")]
        timezone: Option<String>,
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

/// `--catchup` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CatchupArg {
    /// Run the most recent missed occurrence only.
    Latest,
    /// Run every missed occurrence, up to `--catchup-max`.
    All,
    /// Run no missed occurrence.
    Skip,
}

impl From<CatchupArg> for CatchupPolicy {
    fn from(arg: CatchupArg) -> Self {
        match arg {
            CatchupArg::Latest => Self::Latest,
            CatchupArg::All => Self::All,
            CatchupArg::Skip => Self::Skip,
        }
    }
}

/// `--overlap` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OverlapArg {
    /// Start another run even if one is still active.
    Allow,
    /// Drop the occurrence while a run of the schedule is still active.
    Skip,
}

impl From<OverlapArg> for OverlapPolicy {
    fn from(arg: OverlapArg) -> Self {
        match arg {
            OverlapArg::Allow => Self::Allow,
            OverlapArg::Skip => Self::Skip,
        }
    }
}

/// The catch-up policy, with its bound under `all`.
fn catchup_label(s: &ScheduleResponse) -> String {
    match s.catchup {
        CatchupPolicy::All => format!("all (max {})", s.catchup_max),
        CatchupPolicy::Latest => "latest".to_string(),
        CatchupPolicy::Skip => "skip".to_string(),
    }
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
        "PRIORITY",
        "TIMEZONE",
        "CATCHUP",
        "OVERLAP",
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
            s.priority.to_string(),
            s.timezone.clone(),
            catchup_label(s),
            format!("{:?}", s.overlap).to_lowercase(),
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
            priority,
            catchup,
            catchup_max,
            catchup_window_secs,
            overlap,
            timezone,
        } => {
            let parsed_inputs: serde_json::Value =
                serde_json::from_str(inputs).map_err(|e| anyhow::anyhow!("invalid JSON: {e}"))?;
            let response = client
                .create_schedule(&CreateScheduleRequest {
                    workflow_name: workflow.clone(),
                    cron_expression: cron.clone(),
                    inputs: Some(parsed_inputs),
                    priority: priority.map(i32::from),
                    catchup: catchup.map(CatchupPolicy::from),
                    catchup_max: *catchup_max,
                    catchup_window_secs: *catchup_window_secs,
                    overlap: overlap.map(OverlapPolicy::from),
                    timezone: timezone.clone(),
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
            "priority": -10,
            "catchup": "latest",
            "catchup_max": 10,
            "catchup_window_secs": 86400,
            "overlap": "allow",
            "timezone": "UTC",
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

    #[test]
    fn schedules_table_shows_the_priority() {
        let output = schedules_table(&[schedule(None, None)]).to_string();
        assert!(
            output.contains("PRIORITY"),
            "header missing from:\n{output}"
        );
        assert!(output.contains("-10"), "priority missing from:\n{output}");
    }

    #[test]
    fn schedules_table_shows_timezone_and_policies() {
        let mut s = schedule(None, None);
        s.timezone = "Europe/Paris".to_string();
        s.catchup = CatchupPolicy::All;
        s.catchup_max = 24;
        s.overlap = OverlapPolicy::Skip;

        let output = schedules_table(&[s]).to_string();

        for header in ["TIMEZONE", "CATCHUP", "OVERLAP"] {
            assert!(output.contains(header), "{header} missing from:\n{output}");
        }
        assert!(
            output.contains("Europe/Paris"),
            "timezone missing from:\n{output}"
        );
        assert!(
            output.contains("all (max 24)"),
            "catchup missing from:\n{output}"
        );
        assert!(output.contains("skip"), "overlap missing from:\n{output}");
    }

    #[test]
    fn catchup_label_names_the_policy() {
        let mut s = schedule(None, None);
        assert_eq!(catchup_label(&s), "latest");
        s.catchup = CatchupPolicy::Skip;
        assert_eq!(catchup_label(&s), "skip");
    }
}
