//! [`AuditLogStore`] trait implementation for [`PostgresStore`].

use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::audit_log_store::AuditLogStore;
use crate::entities::{AuditLogEntry, AuditLogFilter, EventKind, NewAuditLogEntry, Page};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::PostgresStore;

/// Row struct for the `append_audit_log` RETURNING clause.
struct AuditLogRow {
    id: Uuid,
    event_type: String,
    payload: serde_json::Value,
    run_id: Option<Uuid>,
    step_id: Option<Uuid>,
    user_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

impl AuditLogRow {
    fn into_entry(self) -> Result<AuditLogEntry, StoreError> {
        let event_type = EventKind::from_str(&self.event_type)
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(AuditLogEntry {
            id: self.id,
            event_type,
            payload: self.payload,
            run_id: self.run_id,
            step_id: self.step_id,
            user_id: self.user_id,
            created_at: self.created_at,
        })
    }
}

fn row_to_entry(row: sqlx::postgres::PgRow) -> Result<AuditLogEntry, StoreError> {
    let event_type_str: String = row.get("event_type");
    let event_type =
        EventKind::from_str(&event_type_str).map_err(|e| StoreError::Database(e.to_string()))?;

    Ok(AuditLogEntry {
        id: row.get("id"),
        event_type,
        payload: row.get("payload"),
        run_id: row.get("run_id"),
        step_id: row.get("step_id"),
        user_id: row.get("user_id"),
        created_at: row.get("created_at"),
    })
}

impl AuditLogStore for PostgresStore {
    fn append_audit_log(&self, entry: NewAuditLogEntry) -> StoreFuture<'_, AuditLogEntry> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();

            let row = sqlx::query_as!(
                AuditLogRow,
                r#"
                INSERT INTO ironflow.audit_logs (id, event_type, payload, run_id, step_id, user_id, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id, event_type, payload, run_id, step_id, user_id, created_at
                "#,
                id,
                entry.event_type.as_str(),
                &entry.payload,
                entry.run_id,
                entry.step_id,
                entry.user_id,
                now,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            row.into_entry()
        })
    }

    fn list_audit_logs(
        &self,
        filter: AuditLogFilter,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<AuditLogEntry>> {
        Box::pin(async move {
            let offset = (page.saturating_sub(1) as i64) * (per_page as i64);

            let mut conditions = Vec::new();
            let mut bind_idx = 1u32;

            if filter.event_type.is_some() {
                conditions.push(format!("event_type = ${bind_idx}"));
                bind_idx += 1;
            }
            if filter.run_id.is_some() {
                conditions.push(format!("run_id = ${bind_idx}"));
                bind_idx += 1;
            }
            if filter.from.is_some() {
                conditions.push(format!("created_at >= ${bind_idx}"));
                bind_idx += 1;
            }
            if filter.to.is_some() {
                conditions.push(format!("created_at <= ${bind_idx}"));
                bind_idx += 1;
            }

            let where_clause = if conditions.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", conditions.join(" AND "))
            };

            let sql = format!(
                r#"
                SELECT id, event_type, payload, run_id, step_id, user_id, created_at,
                       COUNT(*) OVER() as total_count
                FROM ironflow.audit_logs
                {where_clause}
                ORDER BY created_at DESC
                LIMIT ${bind_idx} OFFSET ${}
                "#,
                bind_idx + 1
            );

            let mut query = sqlx::query(&sql);

            if let Some(ref event_type) = filter.event_type {
                query = query.bind(event_type.as_str());
            }
            if let Some(run_id) = filter.run_id {
                query = query.bind(run_id);
            }
            if let Some(from) = filter.from {
                query = query.bind(from);
            }
            if let Some(to) = filter.to {
                query = query.bind(to);
            }

            query = query.bind(per_page as i64).bind(offset);

            let rows = query
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let total = if rows.is_empty() {
                0u64
            } else {
                rows[0].get::<i64, _>("total_count") as u64
            };

            let items = rows
                .into_iter()
                .map(row_to_entry)
                .collect::<Result<Vec<_>, _>>()?;

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }
}
