use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::approval_delegation_store::ApprovalDelegationStore;
use crate::entities::{ApprovalDelegation, DelegationFilter, NewApprovalDelegation, Page};
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

struct DelegationRowWithTotal {
    id: Uuid,
    from_user_id: Uuid,
    to_user_id: Uuid,
    valid_from: DateTime<Utc>,
    valid_until: DateTime<Utc>,
    workflow_filter: Option<String>,
    created_at: DateTime<Utc>,
    total_count: i64,
}

impl From<DelegationRowWithTotal> for ApprovalDelegation {
    fn from(row: DelegationRowWithTotal) -> Self {
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
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<ApprovalDelegation>> {
        Box::pin(async move {
            let now = Utc::now();
            let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
            // The null-or-equal idiom lets a single prepared statement serve
            // every combination of the optional filters.
            let rows = sqlx::query_as!(
                DelegationRowWithTotal,
                r#"
                SELECT id, from_user_id, to_user_id, valid_from, valid_until,
                    workflow_filter, created_at,
                    COUNT(*) OVER() AS "total_count!: i64"
                FROM ironflow.approval_delegations
                WHERE valid_from <= $1 AND valid_until > $1
                  AND ($2::uuid IS NULL OR from_user_id = $2)
                  AND ($3::uuid IS NULL OR to_user_id = $3)
                  AND ($4::uuid IS NULL OR from_user_id = $4 OR to_user_id = $4)
                ORDER BY created_at DESC
                LIMIT $5 OFFSET $6
                "#,
                now,
                filter.from_user_id,
                filter.to_user_id,
                filter.involving_user_id,
                per_page as i64,
                offset,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let total = rows.first().map_or(0u64, |r| r.total_count as u64);
            let items = rows.into_iter().map(ApprovalDelegation::from).collect();

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn find_active_delegation(
        &self,
        from_user_id: Uuid,
        to_user_id: Uuid,
        workflow_name: &str,
    ) -> StoreFuture<'_, Option<ApprovalDelegation>> {
        let workflow_name = workflow_name.to_string();
        Box::pin(async move {
            let now = Utc::now();
            // A single pair holds a handful of rows at most. The glob is matched
            // in Rust so both stores share one implementation of it.
            let rows = sqlx::query_as!(
                DelegationRow,
                r#"
                SELECT id, from_user_id, to_user_id, valid_from, valid_until,
                    workflow_filter, created_at
                FROM ironflow.approval_delegations
                WHERE from_user_id = $1 AND to_user_id = $2
                  AND valid_from <= $3 AND valid_until > $3
                ORDER BY created_at DESC
                "#,
                from_user_id,
                to_user_id,
                now,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows
                .into_iter()
                .map(ApprovalDelegation::from)
                .find(|d| d.matches_workflow(&workflow_name)))
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
