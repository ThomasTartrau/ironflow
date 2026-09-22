use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::approval_delegation_store::ApprovalDelegationStore;
use crate::entities::{ApprovalDelegation, DelegationFilter, NewApprovalDelegation};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::PostgresStore;

struct DelegationRow {
    id: Uuid,
    from_user_id: Uuid,
    to_user_id: Uuid,
    valid_from: DateTime<Utc>,
    valid_until: DateTime<Utc>,
    workflow_filter: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<DelegationRow> for ApprovalDelegation {
    fn from(row: DelegationRow) -> Self {
        Self {
            id: row.id,
            from_user_id: row.from_user_id,
            to_user_id: row.to_user_id,
            valid_from: row.valid_from,
            valid_until: row.valid_until,
            workflow_filter: row.workflow_filter,
            created_at: row.created_at,
        }
    }
}

impl ApprovalDelegationStore for PostgresStore {
    fn create_delegation(&self, req: NewApprovalDelegation) -> StoreFuture<'_, ApprovalDelegation> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();
            let row = sqlx::query_as!(
                DelegationRow,
                r#"
                INSERT INTO ironflow.approval_delegations
                    (id, from_user_id, to_user_id, valid_from, valid_until, workflow_filter, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id, from_user_id, to_user_id, valid_from, valid_until, workflow_filter, created_at
                "#,
                id,
                req.from_user_id,
                req.to_user_id,
                req.valid_from,
                req.valid_until,
                req.workflow_filter.as_deref(),
                now,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(ApprovalDelegation::from(row))
        })
    }

    fn find_delegation_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<ApprovalDelegation>> {
        Box::pin(async move {
            let row = sqlx::query_as!(
                DelegationRow,
                r#"
                SELECT id, from_user_id, to_user_id, valid_from, valid_until,
                    workflow_filter, created_at
                FROM ironflow.approval_delegations
                WHERE id = $1
                "#,
                id,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(ApprovalDelegation::from))
        })
    }

    fn list_active_delegations(
        &self,
        filter: DelegationFilter,
    ) -> StoreFuture<'_, Vec<ApprovalDelegation>> {
        Box::pin(async move {
            let now = Utc::now();
            // The null-or-equal idiom lets a single prepared statement serve
            // every combination of the two optional filters.
            let rows = sqlx::query_as!(
                DelegationRow,
                r#"
                SELECT id, from_user_id, to_user_id, valid_from, valid_until,
                    workflow_filter, created_at
                FROM ironflow.approval_delegations
                WHERE valid_from <= $1 AND valid_until > $1
                  AND ($2::uuid IS NULL OR from_user_id = $2)
                  AND ($3::uuid IS NULL OR to_user_id = $3)
                ORDER BY created_at DESC
                "#,
                now,
                filter.from_user_id,
                filter.to_user_id,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows.into_iter().map(ApprovalDelegation::from).collect())
        })
    }

    fn delete_delegation(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let result = sqlx::query!(
                "DELETE FROM ironflow.approval_delegations WHERE id = $1",
                id,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            if result.rows_affected() == 0 {
                return Err(StoreError::DelegationNotFound(id));
            }
            Ok(())
        })
    }
}
