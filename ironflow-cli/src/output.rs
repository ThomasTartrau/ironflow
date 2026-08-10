//! Output formatting for table and JSON modes.
//!
//! Provides helpers to render API responses as either a UTF-8 styled
//! terminal table (with colored status) or raw JSON.

use std::io::{Write, stdout};

use anyhow::Result;
use chrono::{DateTime, Utc};
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};
use ironflow_sdk::types::{
    ApiKeyResponse, ApiKeyScope, ArtifactResponse, AuditLogEntry, CreateApiKeyResponse,
    KeyVersionsResponse, RunDetailResponse, RunResponse, RunStatus, ScopeEntry, SecretResponse,
    StatsResponse, StepResponse, StepStatus, UserResponse, WorkflowDetailResponse, WorkflowSummary,
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
        "Attempt",
        "Duration",
        "Cost",
        "Artifacts",
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

        table.add_row(vec![
            Cell::new(key.id),
            Cell::new(&key.name),
            Cell::new(&key.key_prefix),
            Cell::new(format_scopes(&key.scopes)),
            active,
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

    use ironflow_sdk::types::{ApiKeyScope, CreatedBy, CreatedByKind, EventKind, TriggerKind};
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
        assert!(output.contains('-'), "{output}");
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
}
