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
/// Fraction of the cost cap above which the spend is highlighted.
const COST_WARNING_RATIO: f64 = 0.8;

/// Render a run's spend, with its cap when one is configured.
///
/// Without a cap this is the plain amount; with one it reads `$0.1800 / $2.00`.
fn format_cost(cost_usd: f64, max_cost_usd: Option<f64>) -> String {
    match max_cost_usd {
        Some(cap) => format!("${cost_usd:.4} / ${cap:.2}"),
        None => format!("${cost_usd:.4}"),
    }
}

/// Highlight colour for a run's spend relative to its cap.
///
/// `None` means no highlight: either the run has no cap, or it is comfortably
/// below it. Yellow past [`COST_WARNING_RATIO`] of the cap, red once the cap is
/// reached. A zero cap has no meaningful ratio, so any spend counts as reached.
fn cost_color(cost_usd: f64, max_cost_usd: Option<f64>) -> Option<Color> {
    let cap = max_cost_usd?;

    if cap <= 0.0 {
        return (cost_usd > 0.0).then_some(Color::Red);
    }

    let ratio = cost_usd / cap;
    if ratio >= 1.0 {
        Some(Color::Red)
    } else if ratio >= COST_WARNING_RATIO {
        Some(Color::Yellow)
    } else {
        None
    }
}

/// Build the table cell for a run's spend, highlighted when close to its cap.
fn cost_cell(cost_usd: f64, max_cost_usd: Option<f64>) -> Cell {
    let cell = Cell::new(format_cost(cost_usd, max_cost_usd));
    match cost_color(cost_usd, max_cost_usd) {
        Some(color) => cell.fg(color),
        None => cell,
    }
}

pub fn runs_table(runs: &[RunResponse]) -> Table {
    let mut table = base_table();
    table.set_header(vec![
        "ID",
        "Workflow",
        "Status",
        "Triggered by",
        "Duration",
        "Cost",
        "Created",
        "Started",
    ]);

    for run in runs {
        let status_cell = Cell::new(run.status)
            .fg(status_color(&run.status))
            .set_alignment(CellAlignment::Center);

        table.add_row(vec![
            Cell::new(run.id.to_string().split('-').next().unwrap_or("")),
            Cell::new(&run.workflow_name),
            status_cell,
            Cell::new(&run.created_by.label),
            Cell::new(format_duration_ms(run.duration_ms)),
            cost_cell(run.cost_usd, run.max_cost_usd),
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
        Cell::new("Triggered by"),
        Cell::new(&run.created_by.label),
    ]);
    table.add_row(vec![
        Cell::new("Duration"),
        Cell::new(format_duration_ms(run.duration_ms)),
    ]);
    table.add_row(vec![
        Cell::new("Cost"),
        cost_cell(run.cost_usd, run.max_cost_usd),
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
        "Attempt",
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
            Cell::new(step.attempt).set_alignment(CellAlignment::Center),
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
    use std::collections::HashMap;
    use std::slice;

    use ironflow_sdk::types::{CreatedBy, CreatedByKind, TriggerKind};
    use serde_json::{Map, Value};
    use uuid::Uuid;

    use super::*;

    /// Minimal run whose only meaningful field is its author.
    fn run_fixture(created_by: CreatedBy) -> RunResponse {
        let now = Utc::now();
        RunResponse {
            id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            status: RunStatus::Completed,
            trigger: TriggerKind::Api,
            error: None,
            retry_count: 0,
            max_retries: 0,
            cost_usd: 0.0,
            duration_ms: 0,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by,
            idempotency_key: None,
            max_cost_usd: None,
        }
    }

    #[test]
    fn format_cost_without_cap_shows_amount_only() {
        assert_eq!(format_cost(0.1234, None), "$0.1234");
    }

    #[test]
    fn format_cost_with_cap_shows_both_amounts() {
        assert_eq!(format_cost(0.18, Some(2.0)), "$0.1800 / $2.00");
    }

    #[test]
    fn cost_color_is_absent_without_a_cap() {
        assert_eq!(cost_color(999.0, None), None);
    }

    #[test]
    fn cost_color_warns_past_the_threshold_and_alerts_at_the_cap() {
        assert_eq!(cost_color(1.0, Some(2.0)), None); // 50%
        assert_eq!(cost_color(1.6, Some(2.0)), Some(Color::Yellow)); // 80%
        assert_eq!(cost_color(1.99, Some(2.0)), Some(Color::Yellow));
        assert_eq!(cost_color(2.0, Some(2.0)), Some(Color::Red)); // at cap
        assert_eq!(cost_color(2.5, Some(2.0)), Some(Color::Red)); // over cap
    }

    #[test]
    fn cost_color_handles_a_zero_cap() {
        assert_eq!(cost_color(0.0, Some(0.0)), None);
        assert_eq!(cost_color(0.01, Some(0.0)), Some(Color::Red));
    }

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
        assert!(output.contains("Triggered by"));
    }

    #[test]
    fn runs_table_renders_the_author_label() {
        let run = run_fixture(CreatedBy {
            kind: CreatedByKind::ApiKey,
            id: Some(Uuid::now_v7()),
            label: "ci-deploy (alice)".to_string(),
        });

        let output = runs_table(slice::from_ref(&run)).to_string();
        assert!(
            output.contains("ci-deploy (alice)"),
            "author missing from:\n{output}"
        );
    }

    #[test]
    fn run_detail_table_renders_the_author_label() {
        let detail = RunDetailResponse {
            run: run_fixture(CreatedBy {
                kind: CreatedByKind::System,
                id: None,
                label: "/hooks/github".to_string(),
            }),
            steps: Vec::new(),
            payload: Value::Object(Map::new()),
        };

        let output = run_detail_table(&detail).to_string();
        assert!(output.contains("Triggered by"));
        assert!(
            output.contains("/hooks/github"),
            "author missing from:\n{output}"
        );
    }

    #[test]
    fn empty_workflows_table_has_header() {
        let table = workflows_table(&[]);
        let output = table.to_string();
        assert!(output.contains("Name"));
        assert!(output.contains("Category"));
    }
}
