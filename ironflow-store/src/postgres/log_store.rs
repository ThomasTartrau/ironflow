//! [`LogStore`] trait implementation for [`PostgresStore`].

use std::str::FromStr;

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::entities::{LogEntry, LogFilter, LogStream, NewLogEntries};
use crate::error::StoreError;
use crate::log_store::LogStore;
use crate::store::StoreFuture;

use super::PostgresStore;

fn row_to_entry(row: sqlx::postgres::PgRow) -> Result<LogEntry, StoreError> {
    let stream_str: String = row.get("stream");
    let stream =
        LogStream::from_str(&stream_str).map_err(|e| StoreError::Database(e.to_string()))?;

    Ok(LogEntry {
        id: row.get("id"),
        run_id: row.get("run_id"),
        step_id: row.get("step_id"),
        step_name: row.get("step_name"),
        stream,
        line: row.get("line"),
        created_at: row.get("created_at"),
    })
}

impl LogStore for PostgresStore {
    fn append_logs(&self, entries: NewLogEntries) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            if entries.lines.is_empty() {
                return Ok(());
            }

            let now = Utc::now();
            let stream_str = entries.stream.as_str();

            let n = entries.lines.len();
            let ids: Vec<_> = (0..n).map(|_| Uuid::now_v7()).collect();
            let run_ids = vec![entries.run_id; n];
            let step_ids = vec![entries.step_id; n];
            let step_names = vec![entries.step_name.clone(); n];
            let streams = vec![stream_str.to_string(); n];
            let timestamps = vec![now; n];

            sqlx::query!(
                r#"
                INSERT INTO ironflow.run_logs (id, run_id, step_id, step_name, stream, line, created_at)
                SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::text[], $5::text[], $6::text[], $7::timestamptz[])
                "#,
                &ids,
                &run_ids,
                &step_ids,
                &step_names,
                &streams,
                &entries.lines,
                &timestamps,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn get_logs(
        &self,
        run_id: Uuid,
        filter: LogFilter,
        cursor: Option<Uuid>,
        limit: u32,
    ) -> StoreFuture<'_, Vec<LogEntry>> {
        Box::pin(async move {
            let mut conditions = vec!["run_id = $1".to_string()];
            let mut bind_idx = 2u32;

            if filter.step_id.is_some() {
                conditions.push(format!("step_id = ${bind_idx}"));
                bind_idx += 1;
            }
            if filter.stream.is_some() {
                conditions.push(format!("stream = ${bind_idx}"));
                bind_idx += 1;
            }
            if cursor.is_some() {
                conditions.push(format!("id > ${bind_idx}"));
                bind_idx += 1;
            }

            let where_clause = conditions.join(" AND ");

            let sql = format!(
                r#"
                SELECT id, run_id, step_id, step_name, stream, line, created_at
                FROM ironflow.run_logs
                WHERE {where_clause}
                ORDER BY id ASC
                LIMIT ${bind_idx}
                "#
            );

            let mut query = sqlx::query(&sql).bind(run_id);

            if let Some(step_id) = filter.step_id {
                query = query.bind(step_id);
            }
            if let Some(ref stream) = filter.stream {
                query = query.bind(stream.as_str());
            }
            if let Some(cursor) = cursor {
                query = query.bind(cursor);
            }

            query = query.bind(limit as i64);

            let rows = query
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            rows.into_iter().map(row_to_entry).collect()
        })
    }
}
