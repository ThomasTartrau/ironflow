//! Output formatting for table and JSON modes.
//!
//! Provides helpers to render API responses as either a UTF-8 styled
//! terminal table (with colored status) or raw JSON.

use std::io::Write;

use chrono::{DateTime, Utc};
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};
use ironflow_sdk::types::{
    RunDetailResponse, RunResponse, RunStatus, StatsResponse, StepResponse, StepStatus,
    WorkflowDetailResponse, WorkflowSummary,
};
use serde::Serialize;

/// Map a [`RunStatus`] to a terminal color.
fn status_color(status: &RunStatus) -> Color {
    match status {
        RunStatus::Completed => Color::Green,
        RunStatus::Failed => Color::Red,
        RunStatus::Running => Color::Blue,
        RunStatus::Pending => Color::Yellow,
        RunStatus::Cancelled => Color::Grey,
        RunStatus::AwaitingApproval => Color::Magenta,
        RunStatus::Retrying => Color::Cyan,
    }
}

/// Map a [`StepStatus`] to a terminal color.
fn step_status_color(status: &StepStatus) -> Color {
    match status {
        StepStatus::Completed => Color::Green,
        StepStatus::Failed => Color::Red,
        StepStatus::Running => Color::Blue,
        StepStatus::Pending => Color::Yellow,
        StepStatus::Skipped => Color::Grey,
        StepStatus::AwaitingApproval => Color::Magenta,
        StepStatus::Rejected => Color::Red,
    }
}

/// Format a [`DateTime`] as `YYYY-MM-DD HH:MM:SS`.
fn format_datetime(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Format an optional [`DateTime`].
fn format_optional_datetime(dt: &Option<DateTime<Utc>>) -> String {
    dt.as_ref().map_or("-".to_string(), format_datetime)
}

/// Format milliseconds as a human-readable duration.
fn format_duration_ms(ms: i64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let secs = ms / 1000;
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    let remaining_secs = secs % 60;
    if mins < 60 {
        return format!("{mins}m {remaining_secs}s");
    }
    let hours = mins / 60;
    let remaining_mins = mins % 60;
    format!("{hours}h {remaining_mins}m")
}

/// Create a base table with UTF-8 styling.
fn base_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

/// Render a value as JSON or table into the given writer.
///
/// # Errors
///
/// Returns an error if JSON serialization or writing fails.
pub fn render_output<W: Write, T: Serialize>(
    writer: &mut W,
    json_mode: bool,
    value: &T,
    table_fn: impl FnOnce() -> Table,
) -> anyhow::Result<()> {
    if json_mode {
        let json = serde_json::to_string_pretty(value)?;
        writeln!(writer, "{json}")?;
    } else {
        writeln!(writer, "{}", table_fn())?;
    }
    Ok(())
}

/// Convenience wrapper: render to stdout.
///
/// # Errors
///
/// Returns an error if JSON serialization or writing fails.
pub fn print_output<T: Serialize>(
    json_mode: bool,
    value: &T,
    table_fn: impl FnOnce() -> Table,
) -> anyhow::Result<()> {
    render_output(&mut std::io::stdout().lock(), json_mode, value, table_fn)
}

/// Render a list of runs as a table.
pub fn runs_table(runs: &[RunResponse]) -> Table {
    let mut table = base_table();
    table.set_header(vec![
        "ID", "Workflow", "Status", "Duration", "Cost", "Created", "Started",
    ]);

    for run in runs {
        let status_cell = Cell::new(run.status)
            .fg(status_color(&run.status))
            .set_alignment(CellAlignment::Center);

        table.add_row(vec![
            Cell::new(run.id.to_string().split('-').next().unwrap_or("")),
            Cell::new(&run.workflow_name),
            status_cell,
            Cell::new(format_duration_ms(run.duration_ms)),
            Cell::new(format!("${:.4}", run.cost_usd)),
            Cell::new(format_datetime(&run.created_at)),
            Cell::new(format_optional_datetime(&run.started_at)),
        ]);
    }

    table
}

/// Render a single run detail as a table.
pub fn run_detail_table(detail: &RunDetailResponse) -> Table {
    let run = &detail.run;
    let mut table = base_table();
    table.set_header(vec!["Field", "Value"]);

    let status_cell = Cell::new(run.status).fg(status_color(&run.status));

    table.add_row(vec![Cell::new("ID"), Cell::new(run.id)]);
    table.add_row(vec![Cell::new("Workflow"), Cell::new(&run.workflow_name)]);
    table.add_row(vec![Cell::new("Status"), status_cell]);
    table.add_row(vec![
        Cell::new("Trigger"),
        Cell::new(format!("{:?}", run.trigger)),
    ]);
    table.add_row(vec![
        Cell::new("Duration"),
        Cell::new(format_duration_ms(run.duration_ms)),
    ]);
    table.add_row(vec![
        Cell::new("Cost"),
        Cell::new(format!("${:.4}", run.cost_usd)),
    ]);
    table.add_row(vec![
        Cell::new("Created"),
        Cell::new(format_datetime(&run.created_at)),
    ]);
    table.add_row(vec![
        Cell::new("Started"),
        Cell::new(format_optional_datetime(&run.started_at)),
    ]);
    table.add_row(vec![
        Cell::new("Completed"),
        Cell::new(format_optional_datetime(&run.completed_at)),
    ]);
    table.add_row(vec![
        Cell::new("Retries"),
        Cell::new(format!("{}/{}", run.retry_count, run.max_retries)),
    ]);

    if let Some(ref error) = run.error {
        table.add_row(vec![Cell::new("Error"), Cell::new(error).fg(Color::Red)]);
    }

    if !detail.steps.is_empty() {
        table.add_row(vec![
            Cell::new("Steps"),
            Cell::new(format!("{} step(s)", detail.steps.len())),
        ]);
    }

    table
}

/// Render a run's steps as a table.
pub fn steps_table(steps: &[StepResponse]) -> Table {
    let mut table = base_table();
    table.set_header(vec![
        "ID",
        "Name",
        "Status",
        "Duration",
        "Cost",
        "Started",
        "Completed",
    ]);

    for step in steps {
        let color = step_status_color(&step.status);

        table.add_row(vec![
            Cell::new(step.id.to_string().split('-').next().unwrap_or("")),
            Cell::new(&step.name),
            Cell::new(step.status)
                .fg(color)
                .set_alignment(CellAlignment::Center),
            Cell::new(format_duration_ms(step.duration_ms)),
            Cell::new(format!("${:.4}", step.cost_usd)),
            Cell::new(format_optional_datetime(&step.started_at)),
            Cell::new(format_optional_datetime(&step.completed_at)),
        ]);
    }

    table
}

/// Render a list of workflows as a table.
pub fn workflows_table(workflows: &[WorkflowSummary]) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Name", "Category", "Version"]);

    for wf in workflows {
        table.add_row(vec![
            Cell::new(&wf.name),
            Cell::new(wf.category.as_deref().unwrap_or("-")),
            Cell::new(wf.version.as_deref().unwrap_or("-")),
        ]);
    }

    table
}

/// Render a workflow detail as a table.
pub fn workflow_detail_table(detail: &WorkflowDetailResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Field", "Value"]);

    table.add_row(vec![Cell::new("Name"), Cell::new(&detail.name)]);
    table.add_row(vec![
        Cell::new("Description"),
        Cell::new(&detail.description),
    ]);
    table.add_row(vec![
        Cell::new("Category"),
        Cell::new(detail.category.as_deref().unwrap_or("-")),
    ]);
    table.add_row(vec![
        Cell::new("Version"),
        Cell::new(detail.version.as_deref().unwrap_or("-")),
    ]);

    if !detail.sub_workflows.is_empty() {
        let names: Vec<&str> = detail
            .sub_workflows
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        table.add_row(vec![
            Cell::new("Sub-workflows"),
            Cell::new(names.join(", ")),
        ]);
    }

    table
}

/// Render stats as a table.
pub fn stats_table(stats: &StatsResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Metric", "Value"]);

    table.add_row(vec![Cell::new("Total runs"), Cell::new(stats.total_runs)]);
    table.add_row(vec![
        Cell::new("Completed"),
        Cell::new(stats.completed_runs).fg(Color::Green),
    ]);
    table.add_row(vec![
        Cell::new("Failed"),
        Cell::new(stats.failed_runs).fg(Color::Red),
    ]);
    table.add_row(vec![
        Cell::new("Cancelled"),
        Cell::new(stats.cancelled_runs).fg(Color::Grey),
    ]);
    table.add_row(vec![
        Cell::new("Active"),
        Cell::new(stats.active_runs).fg(Color::Blue),
    ]);
    table.add_row(vec![
        Cell::new("Success rate"),
        Cell::new(format!("{:.1}%", stats.success_rate_percent)),
    ]);
    table.add_row(vec![
        Cell::new("Total cost"),
        Cell::new(format!("${:.4}", stats.total_cost_usd)),
    ]);
    table.add_row(vec![
        Cell::new("Total duration"),
        Cell::new(format_duration_ms(stats.total_duration_ms)),
    ]);

    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_ms_millis() {
        assert_eq!(format_duration_ms(500), "500ms");
        assert_eq!(format_duration_ms(0), "0ms");
    }

    #[test]
    fn format_duration_ms_seconds() {
        assert_eq!(format_duration_ms(5000), "5s");
        assert_eq!(format_duration_ms(59000), "59s");
    }

    #[test]
    fn format_duration_ms_minutes() {
        assert_eq!(format_duration_ms(60000), "1m 0s");
        assert_eq!(format_duration_ms(125000), "2m 5s");
    }

    #[test]
    fn format_duration_ms_hours() {
        assert_eq!(format_duration_ms(3_600_000), "1h 0m");
        assert_eq!(format_duration_ms(5_400_000), "1h 30m");
    }

    #[test]
    fn format_optional_datetime_none() {
        assert_eq!(format_optional_datetime(&None), "-");
    }

    #[test]
    fn format_optional_datetime_some() {
        let dt = "2026-06-02T14:30:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(format_optional_datetime(&Some(dt)), "2026-06-02 14:30:00");
    }

    #[test]
    fn status_colors_are_distinct() {
        let statuses = [
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Running,
            RunStatus::Pending,
            RunStatus::Cancelled,
            RunStatus::AwaitingApproval,
            RunStatus::Retrying,
        ];

        let colors: Vec<Color> = statuses.iter().map(status_color).collect();
        for (i, c1) in colors.iter().enumerate() {
            for (j, c2) in colors.iter().enumerate() {
                if i != j {
                    assert_ne!(c1, c2, "status colors must be distinct");
                }
            }
        }
    }

    #[test]
    fn empty_runs_table_has_header() {
        let table = runs_table(&[]);
        let output = table.to_string();
        assert!(output.contains("ID"));
        assert!(output.contains("Workflow"));
        assert!(output.contains("Status"));
    }

    #[test]
    fn empty_workflows_table_has_header() {
        let table = workflows_table(&[]);
        let output = table.to_string();
        assert!(output.contains("Name"));
        assert!(output.contains("Category"));
    }
}
