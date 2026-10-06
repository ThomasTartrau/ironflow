//! Output of `ironflow run pause`, `ironflow run resume`,
//! `ironflow workflow pause` and `ironflow workflow resume`.

use comfy_table::{Cell, CellAlignment, Table};
use ironflow_sdk::types::{PauseRunResponse, ResumeRunResponse, WorkflowPauseResponse};

use super::{base_table, format_optional_datetime, status_color};

/// Render a paused run, the state it resumes to, and the number of sub-runs
/// paused along with it.
pub fn paused_table(pause: &PauseRunResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec![
        "ID",
        "Workflow",
        "Status",
        "Resumes to",
        "Sub-runs paused",
    ]);
    let resumes_to = pause
        .resume_status
        .map_or_else(|| "-".to_string(), |status| status.to_string());
    table.add_row(vec![
        Cell::new(pause.id),
        Cell::new(&pause.workflow_name),
        Cell::new(pause.status)
            .fg(status_color(&pause.status))
            .set_alignment(CellAlignment::Center),
        Cell::new(resumes_to),
        Cell::new(pause.paused_descendants.len()),
    ]);
    table
}

/// Render a resumed run with the number of sub-runs resumed along with it.
pub fn resumed_table(resume: &ResumeRunResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec!["ID", "Workflow", "Status", "Sub-runs resumed"]);
    table.add_row(vec![
        Cell::new(resume.id),
        Cell::new(&resume.workflow_name),
        Cell::new(resume.status)
            .fg(status_color(&resume.status))
            .set_alignment(CellAlignment::Center),
        Cell::new(resume.resumed_descendants.len()),
    ]);
    table
}

/// Render the pause state of a workflow.
pub fn workflow_pause_table(pause: &WorkflowPauseResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec!["Workflow", "Paused", "Paused at"]);
    table.add_row(vec![
        Cell::new(&pause.workflow_name),
        Cell::new(if pause.paused { "yes" } else { "no" }),
        Cell::new(format_optional_datetime(&pause.paused_at)),
    ]);
    table
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, from_value, json};
    use uuid::Uuid;

    use super::*;

    fn run_fields(id: Uuid, status: &str) -> Value {
        json!({
            "id": id,
            "workflow_name": "deploy",
            "status": status,
            "trigger": {"kind": "manual"},
            "retry_count": 0,
            "max_retries": 0,
            "cost_usd": 0.0,
            "duration_ms": 0,
            "created_at": "2026-10-06T10:00:00Z",
            "updated_at": "2026-10-06T10:00:00Z",
            "created_by": {"kind": "system", "label": "cron"},
        })
    }

    fn row_of(output: &str) -> &str {
        output
            .lines()
            .find(|line| line.contains("deploy"))
            .expect("the row")
    }

    #[test]
    fn paused_table_shows_the_resume_target_and_the_sub_runs_paused() {
        let id = Uuid::now_v7();
        let mut fields = run_fields(id, "paused");
        fields["resume_status"] = json!("sleeping");
        fields["paused_descendants"] = json!([Uuid::now_v7()]);
        let pause: PauseRunResponse = from_value(fields).expect("a pause response");

        let output = paused_table(&pause).to_string();

        assert!(output.contains("Resumes to"), "{output}");
        assert!(output.contains(&id.to_string()), "{output}");
        let row = row_of(&output);
        assert!(row.contains("sleeping"), "{row}");
        assert!(row.contains(" 1 "), "{row}");
    }

    #[test]
    fn resumed_table_shows_the_sub_runs_resumed() {
        let id = Uuid::now_v7();
        let mut fields = run_fields(id, "pending");
        fields["resumed_descendants"] = json!([Uuid::now_v7(), Uuid::now_v7()]);
        let resume: ResumeRunResponse = from_value(fields).expect("a resume response");

        let output = resumed_table(&resume).to_string();

        assert!(output.contains("Sub-runs resumed"), "{output}");
        let row = row_of(&output);
        assert!(row.contains("pending"), "{row}");
        assert!(row.contains(" 2 "), "{row}");
    }

    #[test]
    fn workflow_pause_table_shows_when_the_workflow_was_paused() {
        let pause: WorkflowPauseResponse = from_value(json!({
            "workflow_name": "deploy",
            "paused": true,
            "paused_at": "2026-10-06T10:00:00Z",
        }))
        .expect("a workflow pause response");

        let output = workflow_pause_table(&pause).to_string();

        let row = row_of(&output);
        assert!(row.contains("yes"), "{row}");
        assert!(row.contains("2026-10-06 10:00:00"), "{row}");
    }

    #[test]
    fn workflow_pause_table_shows_a_resumed_workflow() {
        let pause: WorkflowPauseResponse = from_value(json!({
            "workflow_name": "deploy",
            "paused": false,
        }))
        .expect("a workflow pause response");

        let output = workflow_pause_table(&pause).to_string();

        let row = row_of(&output);
        assert!(row.contains("no"), "{row}");
        assert!(row.contains('-'), "{row}");
    }
}
