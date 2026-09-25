//! Output formatting for table and JSON modes.
//!
//! Provides helpers to render API responses as either a UTF-8 styled
//! terminal table (with colored status) or raw JSON.

use std::io::{Write, stdout};

use anyhow::Result;
use chrono::{DateTime, Utc};
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};
use ironflow_sdk::client::ApiResponse;
use ironflow_sdk::types::{
    ApiKeyResponse, ApiKeyScope, ArtifactResponse, AuditLogEntry, CreateApiKeyResponse,
    ExecutionPlanResponse, KeyVersionsResponse, PlannedStepResponse, RunDetailResponse,
    RunResponse, RunStatus, ScopeEntry, SecretResponse, StatsHistoryResponse, StatsResponse,
    StepResponse, StepStatus, UserGroupsResponse, UserResponse, WorkflowDetailResponse,
    WorkflowSummary,
};
use serde::Serialize;
use serde_json::to_string_pretty;
use uuid::Uuid;

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
        RunStatus::Warning => Color::DarkYellow,
        RunStatus::Sleeping => Color::DarkCyan,
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

/// Fraction of the original SLA window below which the countdown turns yellow.
const SLA_WARNING_RATIO: f64 = 0.1;

/// Format a countdown in seconds as a coarse duration.
///
/// `None` renders as `"-"` (no deadline), a non-positive count as `"expired"`.
fn format_remaining_secs(remaining: Option<i64>) -> String {
    let Some(remaining) = remaining else {
        return "-".to_string();
    };
    if remaining <= 0 {
        return "expired".to_string();
    }

    if remaining < 60 {
        return format!("{remaining}s");
    }

    let minutes = remaining / 60;
    if minutes < 60 {
        let rest = remaining % 60;
        return if rest == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {rest}s")
        };
    }

    let hours = minutes / 60;
    let rest = minutes % 60;
    if rest == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {rest}m")
    }
}

/// Colour for a countdown: red once expired, yellow in the last
/// [`SLA_WARNING_RATIO`] of the window, plain otherwise.
fn remaining_color(remaining: Option<i64>, window_secs: Option<i64>) -> Option<Color> {
    let remaining = remaining?;
    if remaining <= 0 {
        return Some(Color::Red);
    }

    let window = window_secs?;
    if window > 0 && (remaining as f64) < (window as f64) * SLA_WARNING_RATIO {
        return Some(Color::Yellow);
    }

    None
}

/// Format the remaining SLA of an approval gate.
///
/// Returns `"-"` for a step without a deadline, `"expired"` once the countdown
/// reaches zero, and a coarse duration (`"45s"`, `"12m 30s"`, `"1h 12m"`)
/// otherwise.
fn format_sla(step: &StepResponse) -> String {
    format_remaining_secs(step.approval_seconds_remaining)
}

/// Colour of the SLA cell.
///
/// The window is derived from the gate's own timestamps (`started_at` to
/// `approval_deadline_at`), so no configuration parsing is needed.
fn sla_color(step: &StepResponse) -> Option<Color> {
    let window = match (step.approval_deadline_at, step.started_at) {
        (Some(deadline), Some(started)) => Some((deadline - started).num_seconds()),
        _ => None,
    };
    remaining_color(step.approval_seconds_remaining, window)
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
) -> Result<()> {
    if json_mode {
        let json = to_string_pretty(value)?;
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
) -> Result<()> {
    render_output(&mut stdout().lock(), json_mode, value, table_fn)
}

/// Render a value as pretty JSON to stdout.
///
/// For commands whose output is a summary the CLI builds itself, with no
/// table equivalent.
///
/// # Errors
///
/// Returns an error if JSON serialization or writing fails.
pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let json = to_string_pretty(value)?;
    writeln!(stdout().lock(), "{json}")?;
    Ok(())
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

/// Summarize a step's artifacts as a count and a total size.
///
/// A dash when the step produced none, so the column stays scannable.
fn format_artifacts(artifacts: &[ArtifactResponse]) -> String {
    if artifacts.is_empty() {
        return "-".to_string();
    }

    let total: i64 = artifacts.iter().map(|artifact| artifact.size_bytes).sum();
    format!("{} ({})", artifacts.len(), format_bytes(total))
}

/// Human-readable file size, using 1024-based units.
fn format_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    let decimals = if value < 10.0 { 1 } else { 0 };
    format!("{value:.decimals$} {}", UNITS[unit])
}

/// Render a run's steps as a table.
pub fn steps_table(steps: &[StepResponse]) -> Table {
    let mut table = base_table();
    table.set_header(vec![
        "ID",
        "Name",
        "Status",
        "SLA",
        "Attempt",
        "Duration",
        "Cost",
        "Artifacts",
        "Started",
        "Completed",
    ]);

    for step in steps {
        let color = step_status_color(&step.status);

        let mut sla = Cell::new(format_sla(step)).set_alignment(CellAlignment::Center);
        if let Some(sla_fg) = sla_color(step) {
            sla = sla.fg(sla_fg);
        }

        table.add_row(vec![
            Cell::new(step.id.to_string().split('-').next().unwrap_or("")),
            Cell::new(&step.name),
            Cell::new(step.status)
                .fg(color)
                .set_alignment(CellAlignment::Center),
            sla,
            Cell::new(step.attempt).set_alignment(CellAlignment::Center),
            Cell::new(format_duration_ms(step.duration_ms)),
            Cell::new(format!("${:.4}", step.cost_usd)),
            Cell::new(format_artifacts(&step.artifacts)).set_alignment(CellAlignment::Center),
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

/// Render an execution plan as an indented tree.
///
/// One line per step. Members of a parallel wave sit under a `parallel-N`
/// header and are indented one extra level; sub-workflow steps are indented by
/// their depth. A step carrying a condition shows why the planner took that
/// branch.
///
/// # Examples
///
/// ```no_run
/// use ironflow_cli::output::execution_plan_tree;
/// use ironflow_sdk::types::ExecutionPlanResponse;
///
/// # fn example(plan: &ExecutionPlanResponse) {
/// println!("{}", execution_plan_tree(plan));
/// # }
/// ```
pub fn execution_plan_tree(plan: &ExecutionPlanResponse) -> String {
    let mut lines = Vec::new();

    let mut header = format!("workflow {}", plan.workflow);
    if let Some(total) = plan.estimated_duration_ms {
        header.push_str(&format!("  estimated ~{}", format_duration_ms(total)));
    }
    lines.push(header);

    let mut current_group: Option<&str> = None;
    for (index, step) in plan.steps.iter().enumerate() {
        let group = step.parallel_group.as_deref();
        if group != current_group {
            if let Some(name) = group {
                lines.push(format!("{}├─ {name}", indent(depth_of(step))));
            }
            current_group = group;
        }

        let extra = if group.is_some() { "  " } else { "" };
        let branch = if is_last_at_depth(plan, index) {
            "└─ "
        } else {
            "├─ "
        };
        lines.push(format!(
            "{}{extra}{branch}{}",
            indent(depth_of(step)),
            step_label(step)
        ));
    }

    if plan.truncated {
        let reason = plan
            .incomplete_reason
            .as_deref()
            .unwrap_or("the plan was cut short");
        lines.push(format!("plan incomplete: {reason}"));
    }

    lines.join("\n")
}

/// Two spaces per sub-workflow level.
fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

/// Sub-workflow depth of a step as an indent level.
fn depth_of(step: &PlannedStepResponse) -> usize {
    usize::try_from(step.depth).unwrap_or(0)
}

/// Whether no later step sits at the same depth, making this the last branch.
fn is_last_at_depth(plan: &ExecutionPlanResponse, index: usize) -> bool {
    let depth = plan.steps[index].depth;
    !plan.steps[index + 1..].iter().any(|s| s.depth == depth)
}

/// `name [kind] ~duration (condition)` for one planned step.
fn step_label(step: &PlannedStepResponse) -> String {
    let mut label = format!("{} [{}]", step.name, step.kind);

    if let Some(ms) = step.estimated_duration_ms {
        label.push_str(&format!(" ~{}", format_duration_ms(ms)));
    }

    if let Some(condition) = &step.condition {
        let suffix = match condition.state.as_str() {
            "evaluated" => format!(
                " (when {} = {})",
                condition.expression.as_deref().unwrap_or("?"),
                condition.value.unwrap_or(false)
            ),
            "skipped" => format!(
                " (skipped: {})",
                condition.reason.as_deref().unwrap_or("no reason given")
            ),
            _ => format!(
                " (condition unevaluable: {})",
                condition.expression.as_deref().unwrap_or("?")
            ),
        };
        label.push_str(&suffix);
    }

    label
}

/// Print an execution plan as JSON or as a tree.
///
/// # Errors
///
/// Returns an error if serialization or writing fails.
pub fn render_execution_plan<W: Write>(
    writer: &mut W,
    json_mode: bool,
    response: &ApiResponse<ExecutionPlanResponse>,
) -> Result<()> {
    if json_mode {
        let json = to_string_pretty(response)?;
        writeln!(writer, "{json}")?;
    } else {
        writeln!(writer, "{}", execution_plan_tree(&response.data))?;
    }
    Ok(())
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
        Cell::new("Awaiting approval"),
        Cell::new(stats.awaiting_approval_runs).fg(Color::Magenta),
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

/// Render historical stats as a table.
pub fn stats_history_table(history: &StatsHistoryResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec![
        "Time",
        "Completed",
        "Warning",
        "Failed",
        "Cancelled",
        "Active",
        "Success %",
        "Avg (ms)",
        "P95 (ms)",
        "Cost",
    ]);

    for bucket in &history.buckets {
        let active = bucket.pending
            + bucket.running
            + bucket.retrying
            + bucket.awaiting_approval
            + bucket.sleeping;
        table.add_row(vec![
            Cell::new(bucket.time),
            Cell::new(bucket.completed).fg(Color::Green),
            Cell::new(bucket.warning).fg(Color::Yellow),
            Cell::new(bucket.failed).fg(Color::Red),
            Cell::new(bucket.cancelled).fg(Color::Grey),
            Cell::new(active).fg(Color::Blue),
            Cell::new(format_success_rate(bucket.success_rate_percent)),
            Cell::new(bucket.avg_duration_ms),
            Cell::new(bucket.p95_duration_ms),
            Cell::new(format!("${:.4}", bucket.total_cost_usd)),
        ]);
    }

    table
}

/// Render an optional success rate: `-` when the bucket has no finished run.
fn format_success_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "-".to_string(), |r| format!("{r:.1}%"))
}

/// Render a list of key versions as a comma-separated string.
fn format_versions(versions: &[i32]) -> String {
    if versions.is_empty() {
        return "-".to_string();
    }
    versions
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Outcome of a `delete` command.
///
/// The API answers `204 No Content`, which serializes to nothing useful, so the
/// CLI reports the deletion itself and keeps `--json` machine-readable.
///
/// # Examples
///
/// ```
/// use ironflow_cli::output::Deleted;
///
/// let deleted = Deleted::new("secret", "db/password");
/// assert_eq!(deleted.kind, "secret");
/// ```
#[derive(Debug, Serialize)]
pub struct Deleted {
    /// What was deleted (`secret`, `api-key`, `user`).
    pub kind: &'static str,
    /// Identifier of the deleted resource.
    pub id: String,
    /// Always `true`; present so consumers can match on a stable shape.
    pub deleted: bool,
}

impl Deleted {
    /// Build a deletion report.
    pub fn new(kind: &'static str, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            deleted: true,
        }
    }
}

/// Render a deletion report as a table.
pub fn deleted_table(deleted: &Deleted) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Deleted", "ID"]);
    table.add_row(vec![Cell::new(deleted.kind), Cell::new(&deleted.id)]);
    table
}

/// Report a deletion on stdout, as a table or as JSON.
///
/// # Errors
///
/// Returns an error if JSON serialization or writing fails.
///
/// # Examples
///
/// ```no_run
/// use ironflow_cli::output::report_deletion;
///
/// # fn example() -> anyhow::Result<()> {
/// report_deletion(false, "secret", "db/password")?;
/// # Ok(())
/// # }
/// ```
pub fn report_deletion(json_mode: bool, kind: &'static str, id: impl Into<String>) -> Result<()> {
    let deleted = Deleted::new(kind, id);
    print_output(json_mode, &deleted, || deleted_table(&deleted))
}

/// Render a list of secrets as a table.
///
/// [`SecretResponse`] carries no value field, so no secret material can reach
/// this table by construction.
pub fn secrets_table(secrets: &[SecretResponse]) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Key", "Created", "Updated"]);

    for secret in secrets {
        table.add_row(vec![
            Cell::new(&secret.key),
            Cell::new(format_datetime(&secret.created_at)),
            Cell::new(format_datetime(&secret.updated_at)),
        ]);
    }

    table
}

/// Join the scopes of an API key into a single cell value.
fn format_scopes(scopes: &[ApiKeyScope]) -> String {
    scopes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render the encryption key ring status as a table.
pub fn key_versions_table(status: &KeyVersionsResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Property", "Versions"]);

    table.add_row(vec![
        Cell::new("Active"),
        Cell::new(status.active).fg(Color::Green),
    ]);
    table.add_row(vec![
        Cell::new("Configured"),
        Cell::new(format_versions(&status.configured)),
    ]);
    table.add_row(vec![
        Cell::new("In use"),
        Cell::new(format_versions(&status.in_use)),
    ]);
    table.add_row(vec![
        Cell::new("Missing"),
        Cell::new(format_versions(&status.missing)).fg(if status.missing.is_empty() {
            Color::Grey
        } else {
            Color::Red
        }),
    ]);
    table.add_row(vec![
        Cell::new("Retirable"),
        Cell::new(format_versions(&status.retirable)).fg(if status.retirable.is_empty() {
            Color::Grey
        } else {
            Color::Yellow
        }),
    ]);

    table
}

/// Render a list of API keys as a table.
///
/// [`ApiKeyResponse`] never carries the raw key, only its prefix.
pub fn api_keys_table(keys: &[ApiKeyResponse]) -> Table {
    let mut table = base_table();
    table.set_header(vec![
        "ID",
        "Name",
        "Prefix",
        "Scopes",
        "Active",
        "Rate limit",
        "Last used",
        "Expires",
        "Created",
    ]);

    for key in keys {
        let active = Cell::new(if key.is_active { "yes" } else { "no" })
            .fg(if key.is_active {
                Color::Green
            } else {
                Color::Grey
            })
            .set_alignment(CellAlignment::Center);

        let rate_limit = key
            .rate_limit_override
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());

        table.add_row(vec![
            Cell::new(key.id),
            Cell::new(&key.name),
            Cell::new(&key.key_prefix),
            Cell::new(format_scopes(&key.scopes)),
            active,
            Cell::new(rate_limit),
            Cell::new(format_optional_datetime(&key.last_used_at)),
            Cell::new(format_optional_datetime(&key.expires_at)),
            Cell::new(format_datetime(&key.created_at)),
        ]);
    }

    table
}

/// Render a freshly created API key, including its one-time raw secret.
///
/// This is the only place the raw key is ever rendered: the API returns it once
/// at creation and never again, so withholding it would make the command
/// useless.
pub fn created_api_key_table(key: &CreateApiKeyResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Field", "Value"]);

    table.add_row(vec![Cell::new("ID"), Cell::new(key.id)]);
    table.add_row(vec![Cell::new("Name"), Cell::new(&key.name)]);
    table.add_row(vec![
        Cell::new("Key"),
        Cell::new(&key.key).fg(Color::Yellow),
    ]);
    table.add_row(vec![Cell::new("Prefix"), Cell::new(&key.key_prefix)]);
    table.add_row(vec![
        Cell::new("Scopes"),
        Cell::new(format_scopes(&key.scopes)),
    ]);
    if let Some(override_val) = key.rate_limit_override {
        table.add_row(vec![
            Cell::new("Rate limit"),
            Cell::new(format!("{override_val} req/min")),
        ]);
    }
    table.add_row(vec![
        Cell::new("Expires"),
        Cell::new(format_optional_datetime(&key.expires_at)),
    ]);
    table.add_row(vec![
        Cell::new("Created"),
        Cell::new(format_datetime(&key.created_at)),
    ]);

    table
}

/// Render the available API key scopes as a table.
pub fn scopes_table(scopes: &[ScopeEntry]) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Value", "Label", "Description"]);

    for scope in scopes {
        table.add_row(vec![
            Cell::new(&scope.value),
            Cell::new(&scope.label),
            Cell::new(&scope.description),
        ]);
    }

    table
}

/// Render a list of users as a table.
pub fn users_table(users: &[UserResponse]) -> Table {
    let mut table = base_table();
    table.set_header(vec!["ID", "Username", "Email", "Admin", "Created"]);

    for user in users {
        let admin = Cell::new(if user.is_admin { "yes" } else { "no" })
            .fg(if user.is_admin {
                Color::Magenta
            } else {
                Color::Grey
            })
            .set_alignment(CellAlignment::Center);

        table.add_row(vec![
            Cell::new(user.id),
            Cell::new(&user.username),
            Cell::new(&user.email),
            admin,
            Cell::new(format_datetime(&user.created_at)),
        ]);
    }

    table
}

/// Render a user's group memberships.
pub fn user_groups_table(resp: &UserGroupsResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec!["User ID", "Groups"]);

    let groups = if resp.groups.is_empty() {
        "-".to_string()
    } else {
        resp.groups.join(", ")
    };
    table.add_row(vec![Cell::new(resp.user_id), Cell::new(groups)]);

    table
}

/// Render a side-by-side comparison of two runs of the same workflow.
pub fn run_diff_table(a: &RunDetailResponse, b: &RunDetailResponse) -> Table {
    let (ra, rb) = (&a.run, &b.run);
    let mut table = base_table();
    table.set_header(vec![
        "Field",
        &format!("Run {}", short_id(ra.id)),
        &format!("Run {}", short_id(rb.id)),
    ]);

    let row = |f: &str, va: String, vb: String| -> Vec<Cell> {
        let hl = va != vb;
        vec![
            Cell::new(f),
            if hl {
                Cell::new(&va).fg(Color::Yellow)
            } else {
                Cell::new(&va)
            },
            if hl {
                Cell::new(&vb).fg(Color::Yellow)
            } else {
                Cell::new(&vb)
            },
        ]
    };

    table.add_row(row("Status", ra.status.to_string(), rb.status.to_string()));
    table.add_row(row(
        "Duration",
        format_duration_ms(ra.duration_ms),
        format_duration_ms(rb.duration_ms),
    ));
    table.add_row(row(
        "Cost",
        format_cost(ra.cost_usd, ra.max_cost_usd),
        format_cost(rb.cost_usd, rb.max_cost_usd),
    ));
    table.add_row(row(
        "Started",
        format_optional_datetime(&ra.started_at),
        format_optional_datetime(&rb.started_at),
    ));
    table.add_row(row(
        "Completed",
        format_optional_datetime(&ra.completed_at),
        format_optional_datetime(&rb.completed_at),
    ));
    table.add_row(row(
        "Error",
        ra.error.clone().unwrap_or("-".into()),
        rb.error.clone().unwrap_or("-".into()),
    ));
    if a.payload != b.payload {
        table.add_row(row(
            "Payload",
            serde_json::to_string(&a.payload).unwrap_or_default(),
            serde_json::to_string(&b.payload).unwrap_or_default(),
        ));
    }
    for i in 0..a.steps.len().max(b.steps.len()) {
        let (sa, sb) = (a.steps.get(i), b.steps.get(i));
        let name = sa.or(sb).map(|s| s.name.as_str()).unwrap_or("-");
        table.add_row(row(
            &format!("{name} status"),
            sa.map(|s| s.status.to_string()).unwrap_or("-".into()),
            sb.map(|s| s.status.to_string()).unwrap_or("-".into()),
        ));
        table.add_row(row(
            &format!("{name} duration"),
            sa.map(|s| format_duration_ms(s.duration_ms))
                .unwrap_or("-".into()),
            sb.map(|s| format_duration_ms(s.duration_ms))
                .unwrap_or("-".into()),
        ));
        table.add_row(row(
            &format!("{name} cost"),
            sa.map(|s| format!("${:.4}", s.cost_usd))
                .unwrap_or("-".into()),
            sb.map(|s| format!("${:.4}", s.cost_usd))
                .unwrap_or("-".into()),
        ));
    }
    table
}

/// Render a UUID as its first hyphen-separated group, enough to spot a row.
fn short_id(id: Uuid) -> String {
    id.to_string()
        .split('-')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Render a UUID as a short prefix, or `-` when absent.
fn format_optional_id(id: &Option<Uuid>) -> String {
    id.map_or_else(|| "-".to_string(), short_id)
}

/// Render a list of audit log entries as a table.
///
/// The event payload is omitted: it is arbitrary JSON that would wreck the
/// table layout. Use `--json` to get it.
pub fn audit_logs_table(entries: &[AuditLogEntry]) -> Table {
    let mut table = base_table();
    table.set_header(vec!["ID", "Type", "Run", "Step", "User", "Created"]);

    for entry in entries {
        table.add_row(vec![
            Cell::new(short_id(entry.id)),
            Cell::new(entry.event_type.to_string()),
            Cell::new(format_optional_id(&entry.run_id)),
            Cell::new(format_optional_id(&entry.step_id)),
            Cell::new(format_optional_id(&entry.user_id)),
            Cell::new(format_datetime(&entry.created_at)),
        ]);
    }

    table
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::slice;

    use ironflow_sdk::types::{
        ApiKeyScope, ConditionResponse, CreatedBy, CreatedByKind, EventKind, TriggerKind,
    };
    use serde_json::{Map, Value};

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
    fn format_success_rate_renders_dash_when_absent() {
        assert_eq!(format_success_rate(None), "-");
    }

    #[test]
    fn format_success_rate_renders_one_decimal() {
        assert_eq!(format_success_rate(Some(100.0)), "100.0%");
        assert_eq!(format_success_rate(Some(200.0 / 3.0)), "66.7%");
        assert_eq!(format_success_rate(Some(0.0)), "0.0%");
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

    fn artifact(name: &str, size_bytes: i64) -> ArtifactResponse {
        ArtifactResponse {
            id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            name: name.to_string(),
            content_type: "text/plain".to_string(),
            size_bytes,
            sha256: "0".repeat(64),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn format_bytes_keeps_raw_bytes_below_one_kilobyte() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_switches_units_at_each_boundary() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn format_bytes_drops_the_decimal_past_ten() {
        assert_eq!(format_bytes(145_408), "142 KB");
    }

    #[test]
    fn format_artifacts_shows_a_dash_when_there_are_none() {
        assert_eq!(format_artifacts(&[]), "-");
    }

    #[test]
    fn format_artifacts_shows_the_count_and_total_size() {
        let artifacts = vec![artifact("a.txt", 1024), artifact("b.txt", 1024)];
        assert_eq!(format_artifacts(&artifacts), "2 (2.0 KB)");
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
    fn format_sla_without_a_deadline_is_a_dash() {
        assert_eq!(format_remaining_secs(None), "-");
    }

    #[test]
    fn format_sla_reports_an_elapsed_deadline_as_expired() {
        assert_eq!(format_remaining_secs(Some(0)), "expired");
        assert_eq!(format_remaining_secs(Some(-30)), "expired");
    }

    #[test]
    fn format_sla_uses_coarse_units() {
        assert_eq!(format_remaining_secs(Some(45)), "45s");
        assert_eq!(format_remaining_secs(Some(59)), "59s");
        assert_eq!(format_remaining_secs(Some(60)), "1m");
        assert_eq!(format_remaining_secs(Some(750)), "12m 30s");
        assert_eq!(format_remaining_secs(Some(3599)), "59m 59s");
        assert_eq!(format_remaining_secs(Some(3600)), "1h");
        assert_eq!(format_remaining_secs(Some(4320)), "1h 12m");
    }

    #[test]
    fn sla_has_no_colour_without_a_deadline() {
        assert_eq!(remaining_color(None, None), None);
        assert_eq!(remaining_color(None, Some(3600)), None);
    }

    #[test]
    fn sla_turns_red_once_expired() {
        assert_eq!(remaining_color(Some(0), Some(3600)), Some(Color::Red));
        assert_eq!(remaining_color(Some(-1), None), Some(Color::Red));
    }

    #[test]
    fn sla_turns_yellow_in_the_last_tenth_of_the_window() {
        assert_eq!(remaining_color(Some(359), Some(3600)), Some(Color::Yellow));
        assert_eq!(remaining_color(Some(360), Some(3600)), None);
        assert_eq!(remaining_color(Some(3000), Some(3600)), None);
    }

    #[test]
    fn sla_has_no_colour_without_a_measurable_window() {
        assert_eq!(remaining_color(Some(120), None), None);
        assert_eq!(remaining_color(Some(120), Some(0)), None);
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

    // ── Secrets ────────────────────────────────────────────────

    fn secret_fixture(key: &str) -> SecretResponse {
        let now = Utc::now();
        SecretResponse {
            id: Uuid::now_v7(),
            key: key.to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn empty_secrets_table_has_header() {
        let output = secrets_table(&[]).to_string();
        assert!(output.contains("Key"));
        assert!(output.contains("Created"));
        assert!(output.contains("Updated"));
    }

    #[test]
    fn secrets_table_renders_the_key() {
        let secret = secret_fixture("workflows/inbox/gmail_token");
        let output = secrets_table(slice::from_ref(&secret)).to_string();
        assert!(output.contains("workflows/inbox/gmail_token"), "{output}");
    }

    /// The value never even reaches this layer: `SecretResponse` has no such
    /// field. Rendering it as JSON proves the whole payload is value-free.
    #[test]
    fn a_secret_response_carries_no_value_at_all() {
        let secret = secret_fixture("db/password");
        let json = serde_json::to_string(&secret).unwrap();
        assert!(!json.contains("value"), "{json}");
    }

    // ── API keys ───────────────────────────────────────────────

    fn api_key_fixture() -> ApiKeyResponse {
        ApiKeyResponse {
            id: Uuid::now_v7(),
            name: "ci-deploy".to_string(),
            key_prefix: "ifk_abcd".to_string(),
            scopes: vec![ApiKeyScope::RunsRead, ApiKeyScope::RunsWrite],
            is_active: true,
            created_at: Utc::now(),
            expires_at: None,
            last_used_at: None,
            rate_limit_override: None,
        }
    }

    #[test]
    fn empty_api_keys_table_has_header() {
        let output = api_keys_table(&[]).to_string();
        for header in ["ID", "Name", "Prefix", "Scopes", "Active"] {
            assert!(output.contains(header), "missing {header} in {output}");
        }
    }

    #[test]
    fn api_keys_table_joins_the_scopes() {
        let key = api_key_fixture();
        let output = api_keys_table(slice::from_ref(&key)).to_string();
        assert!(output.contains("runs_read, runs_write"), "{output}");
        assert!(output.contains("ifk_abcd"), "{output}");
    }

    #[test]
    fn created_api_key_table_shows_the_raw_key() {
        let created = CreateApiKeyResponse {
            id: Uuid::now_v7(),
            name: "ci-deploy".to_string(),
            key: "ifk_full_raw_key".to_string(),
            key_prefix: "ifk_full".to_string(),
            scopes: vec![ApiKeyScope::Admin],
            created_at: Utc::now(),
            expires_at: None,
            rate_limit_override: None,
        };

        let output = created_api_key_table(&created).to_string();
        assert!(output.contains("ifk_full_raw_key"), "{output}");
    }

    #[test]
    fn empty_scopes_table_has_header() {
        let output = scopes_table(&[]).to_string();
        assert!(output.contains("Value"));
        assert!(output.contains("Description"));
    }

    // ── Users ──────────────────────────────────────────────────

    fn user_fixture(is_admin: bool) -> UserResponse {
        let now = Utc::now();
        UserResponse {
            id: Uuid::now_v7(),
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            is_admin,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn empty_users_table_has_header() {
        let output = users_table(&[]).to_string();
        for header in ["ID", "Username", "Email", "Admin", "Created"] {
            assert!(output.contains(header), "missing {header} in {output}");
        }
    }

    #[test]
    fn users_table_spells_out_the_role() {
        let admin = user_fixture(true);
        assert!(
            users_table(slice::from_ref(&admin))
                .to_string()
                .contains("yes")
        );

        let member = user_fixture(false);
        assert!(
            users_table(slice::from_ref(&member))
                .to_string()
                .contains("no")
        );
    }

    #[test]
    fn user_groups_table_has_header_and_lists_the_groups() {
        let resp = UserGroupsResponse {
            user_id: Uuid::now_v7(),
            groups: vec!["finance".to_string(), "sre".to_string()],
        };
        let output = user_groups_table(&resp).to_string();
        for header in ["User ID", "Groups"] {
            assert!(output.contains(header), "missing {header} in {output}");
        }
        assert!(output.contains(&resp.user_id.to_string()), "{output}");
        assert!(output.contains("finance, sre"), "{output}");
    }

    #[test]
    fn user_groups_table_shows_a_dash_without_groups() {
        let resp = UserGroupsResponse {
            user_id: Uuid::now_v7(),
            groups: Vec::new(),
        };
        let output = user_groups_table(&resp).to_string();
        assert!(output.contains("Groups"), "{output}");
        assert!(output.contains(" - "), "{output}");
        assert!(!output.contains("finance"), "{output}");
    }

    // ── Audit logs ─────────────────────────────────────────────

    #[test]
    fn empty_audit_logs_table_has_header() {
        let output = audit_logs_table(&[]).to_string();
        for header in ["ID", "Type", "Run", "Step", "User", "Created"] {
            assert!(output.contains(header), "missing {header} in {output}");
        }
    }

    #[test]
    fn audit_logs_table_omits_the_payload() {
        let entry = AuditLogEntry {
            id: Uuid::now_v7(),
            event_type: EventKind::RunCreated,
            payload: Value::Object(Map::new()),
            run_id: Some(Uuid::now_v7()),
            step_id: None,
            user_id: None,
            created_at: Utc::now(),
        };

        let output = audit_logs_table(slice::from_ref(&entry)).to_string();
        assert!(output.contains("run_created"), "{output}");
        // Absent IDs collapse to a dash rather than an empty cell.
        assert!(output.contains(" - "), "{output}");
    }

    #[test]
    fn format_optional_id_shortens_and_falls_back() {
        assert_eq!(format_optional_id(&None), "-");
        let id = Uuid::now_v7();
        let short = format_optional_id(&Some(id));
        assert_eq!(short, id.to_string().split('-').next().unwrap());
    }

    // ── Deletions ──────────────────────────────────────────────

    #[test]
    fn deleted_table_reports_the_kind_and_id() {
        let deleted = Deleted::new("secret", "db/password");
        let output = deleted_table(&deleted).to_string();
        assert!(output.contains("secret"), "{output}");
        assert!(output.contains("db/password"), "{output}");

        let json = serde_json::to_string(&deleted).unwrap();
        assert!(json.contains(r#""deleted":true"#), "{json}");
    }

    // ── Execution plans ────────────────────────────────────────

    fn planned_step(name: &str, kind: &str, parallel_group: Option<&str>) -> PlannedStepResponse {
        PlannedStepResponse {
            name: name.to_string(),
            kind: kind.to_string(),
            workflow: "deploy".to_string(),
            depth: 0,
            depends_on: Vec::new(),
            condition: None,
            parallel_group: parallel_group.map(str::to_string),
            estimated_duration_ms: None,
        }
    }

    fn plan_fixture(steps: Vec<PlannedStepResponse>) -> ExecutionPlanResponse {
        ExecutionPlanResponse {
            workflow: "deploy".to_string(),
            steps,
            estimated_duration_ms: None,
            max_depth: 3,
            truncated: false,
            incomplete_reason: None,
        }
    }

    #[test]
    fn execution_plan_tree_lists_step_names_and_kinds() {
        let plan = plan_fixture(vec![
            planned_step("build", "shell", None),
            planned_step("deploy", "shell", None),
        ]);

        let output = execution_plan_tree(&plan);
        assert!(output.contains("workflow deploy"), "{output}");
        assert!(output.contains("build [shell]"), "{output}");
        assert!(output.contains("deploy [shell]"), "{output}");
    }

    #[test]
    fn execution_plan_tree_prints_a_parallel_group_header_once() {
        let plan = plan_fixture(vec![
            planned_step("build", "shell", None),
            planned_step("test", "shell", Some("parallel-1")),
            planned_step("lint", "shell", Some("parallel-1")),
        ]);

        let output = execution_plan_tree(&plan);
        assert_eq!(output.matches("parallel-1").count(), 1, "{output}");
    }

    #[test]
    fn execution_plan_tree_shows_the_estimate_when_present() {
        let mut step = planned_step("build", "shell", None);
        step.estimated_duration_ms = Some(5000);
        let mut plan = plan_fixture(vec![step]);
        plan.estimated_duration_ms = Some(5000);

        let output = execution_plan_tree(&plan);
        assert!(output.contains("estimated ~5s"), "{output}");
        assert!(output.contains("build [shell] ~5s"), "{output}");
    }

    #[test]
    fn execution_plan_tree_marks_conditions() {
        let mut evaluated = planned_step("deploy-prod", "shell", None);
        evaluated.condition = Some(ConditionResponse {
            state: "evaluated".to_string(),
            expression: Some("env == prod".to_string()),
            value: Some(true),
            reason: None,
        });
        let mut skipped = planned_step("deploy-dev", "skip", None);
        skipped.condition = Some(ConditionResponse {
            state: "skipped".to_string(),
            expression: None,
            value: None,
            reason: Some("not prod".to_string()),
        });
        let mut unevaluable = planned_step("notify", "http", None);
        unevaluable.condition = Some(ConditionResponse {
            state: "unevaluable".to_string(),
            expression: Some("build succeeded".to_string()),
            value: None,
            reason: Some("depends on a step output".to_string()),
        });

        let output = execution_plan_tree(&plan_fixture(vec![evaluated, skipped, unevaluable]));
        assert!(output.contains("(when env == prod = true)"), "{output}");
        assert!(output.contains("(skipped: not prod)"), "{output}");
        assert!(
            output.contains("(condition unevaluable: build succeeded)"),
            "{output}"
        );
    }

    #[test]
    fn execution_plan_tree_reports_an_incomplete_plan() {
        let mut plan = plan_fixture(vec![planned_step("build", "shell", None)]);
        plan.truncated = true;
        plan.incomplete_reason = Some("step cap of 1000 reached".to_string());

        let output = execution_plan_tree(&plan);
        assert!(
            output.contains("plan incomplete: step cap of 1000 reached"),
            "{output}"
        );
    }

    #[test]
    fn execution_plan_tree_indents_sub_workflow_steps() {
        let mut nested = planned_step("child-step", "shell", None);
        nested.depth = 1;
        let plan = plan_fixture(vec![planned_step("child", "workflow", None), nested]);

        let output = execution_plan_tree(&plan);
        let nested = output
            .lines()
            .find(|l| l.contains("child-step"))
            .expect("nested line");
        assert!(nested.starts_with("  "), "{nested}");
    }
}
