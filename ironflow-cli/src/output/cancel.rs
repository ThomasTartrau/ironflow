//! Output of `ironflow run cancel`.

use comfy_table::{Cell, CellAlignment, Table};
use ironflow_sdk::types::CancelRunResponse;

use super::{base_table, status_color};

/// Render a cancelled run with the number of sub-runs cancelled along with it.
pub fn cancelled_table(cancel: &CancelRunResponse) -> Table {
    let mut table = base_table();
    table.set_header(vec!["ID", "Workflow", "Status", "Sub-runs cancelled"]);
    table.add_row(vec![
        Cell::new(cancel.id),
        Cell::new(&cancel.workflow_name),
        Cell::new(cancel.status)
            .fg(status_color(&cancel.status))
            .set_alignment(CellAlignment::Center),
        Cell::new(cancel.cancelled_descendants.len()),
    ]);
    table
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json};
    use uuid::Uuid;

    use super::*;

    #[test]
    fn cancelled_table_shows_how_many_sub_runs_were_cancelled() {
        let id = Uuid::now_v7();
        let cancel: CancelRunResponse = from_value(json!({
            "id": id,
            "workflow_name": "deploy",
            "status": "cancelled",
            "trigger": {"kind": "manual"},
            "retry_count": 0,
            "max_retries": 0,
            "cost_usd": 0.0,
            "duration_ms": 0,
            "created_at": "2026-10-06T10:00:00Z",
            "updated_at": "2026-10-06T10:00:00Z",
            "created_by": {"kind": "system", "label": "cron"},
            "cancelled_descendants": [Uuid::now_v7(), Uuid::now_v7()],
        }))
        .expect("a cancel response");

        let output = cancelled_table(&cancel).to_string();

        assert!(output.contains("Sub-runs cancelled"), "{output}");
        assert!(output.contains(&id.to_string()), "{output}");
        let row = output
            .lines()
            .find(|line| line.contains("deploy"))
            .expect("the run row");
        assert!(row.contains(" 2 "), "{row}");
    }
}
