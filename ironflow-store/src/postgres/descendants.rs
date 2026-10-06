//! Descendant lookup of the PostgreSQL store, behind
//! [`RunStore::list_active_descendants`](crate::store::RunStore::list_active_descendants).

use uuid::Uuid;

use crate::entities::{PARENT_RUN_ID_LABEL, Run};
use crate::error::StoreError;

use super::PostgresStore;
use super::helpers::{row_to_run, terminal_run_state_names};

/// Non-terminal descendants of a run, oldest first.
///
/// The recursive part follows the parent label from child to child. `UNION`
/// (not `UNION ALL`) drops an id already reached, so a label cycle ends the
/// recursion instead of looping. Terminal runs are walked through and only
/// filtered out at the end.
///
/// Binds: `$1` the run id, `$2` the same id as text, `$3` the parent label
/// key, `$4` the terminal state names.
const ACTIVE_DESCENDANTS_SQL: &str = r#"
    WITH RECURSIVE tree(id) AS (
        SELECT c.id FROM ironflow.runs c
        WHERE c.labels ->> $3 = $2 AND c.trigger @> '{"kind": "workflow"}'::jsonb
        UNION
        SELECT c.id FROM ironflow.runs c
        JOIN tree t ON c.labels ->> $3 = t.id::text
        WHERE c.trigger @> '{"kind": "workflow"}'::jsonb
    )
    SELECT r.*, ast.name as state_name, cu.username as created_by_username,
           ck.name as created_by_api_key_name
    FROM ironflow.runs r
    JOIN tree t ON t.id = r.id
    JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
    JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
    LEFT JOIN iam.users cu ON cu.id = r.created_by_user_id
    LEFT JOIN iam.api_keys ck ON ck.id = r.created_by_api_key_id
    WHERE r.id <> $1 AND ast.name <> ALL($4::text[])
    ORDER BY r.created_at, r.id
"#;

impl PostgresStore {
    /// The non-terminal descendants of `run_id`, oldest first.
    pub(super) async fn active_descendants(&self, run_id: Uuid) -> Result<Vec<Run>, StoreError> {
        let terminal = terminal_run_state_names();
        let rows = sqlx::query(ACTIVE_DESCENDANTS_SQL)
            .bind(run_id)
            .bind(run_id.to_string())
            .bind(PARENT_RUN_ID_LABEL)
            .bind(&terminal[..])
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        rows.iter().map(row_to_run).collect()
    }
}
