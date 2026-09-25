//! PostgreSQL implementation of [`RunStore::get_stats_history`].
//!
//! [`RunStore::get_stats_history`]: crate::store::RunStore::get_stats_history

use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use sqlx::Row;
use sqlx::postgres::PgRow;

use crate::entities::{StatsHistoryBucket, StatsHistoryFilter};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::PostgresStore;
use super::run_store::{bind_run_filter_params, build_run_filter_conditions};

impl PostgresStore {
    /// Build and execute the stats history aggregation query.
    pub(crate) fn stats_history_impl(
        &self,
        filter: StatsHistoryFilter,
    ) -> StoreFuture<'_, Vec<StatsHistoryBucket>> {
        Box::pin(async move {
            let now = Utc::now();
            let start = now - Duration::hours(filter.period.hours());
            // SAFETY: runtime query because date_trunc requires a literal
            // interval name that cannot be parameterized. `interval` comes from
            // HistoryGranularity::pg_interval() which returns one of three
            // hardcoded strings ("hour", "day", "week") -- no user input.
            let interval = filter.granularity.pg_interval();

            let run_filter = filter.to_run_filter();
            let (where_clause, next_idx) = build_run_filter_conditions(&run_filter);
            let time_condition = format!(
                "r.created_at >= ${next_idx} AND r.created_at < ${}",
                next_idx + 1
            );
            let where_clause = if where_clause.is_empty() {
                format!("WHERE {time_condition}")
            } else {
                format!("{where_clause} AND {time_condition}")
            };

            // Truncating in UTC gives UTC bucket boundaries (Monday-aligned
            // weeks) whatever the session TimeZone is.
            let sql = format!(
                r#"
                SELECT
                    (date_trunc('{interval}', r.created_at AT TIME ZONE 'UTC')
                        AT TIME ZONE 'UTC') AS bucket_time,
                    COUNT(*) FILTER (WHERE ast.name = 'completed') AS completed,
                    COUNT(*) FILTER (WHERE ast.name = 'warning') AS warning,
                    COUNT(*) FILTER (WHERE ast.name = 'failed') AS failed,
                    COUNT(*) FILTER (WHERE ast.name = 'cancelled') AS cancelled,
                    COUNT(*) FILTER (WHERE ast.name = 'pending') AS pending,
                    COUNT(*) FILTER (WHERE ast.name = 'running') AS running,
                    COUNT(*) FILTER (WHERE ast.name = 'retrying') AS retrying,
                    COUNT(*) FILTER (
                        WHERE ast.name = 'awaiting_approval'
                    ) AS awaiting_approval,
                    COUNT(*) FILTER (WHERE ast.name = 'sleeping') AS sleeping,
                    COALESCE(
                        AVG(r.duration_ms) FILTER (
                            WHERE ast.name IN ('completed', 'warning', 'failed')
                            AND r.duration_ms > 0
                        ),
                        0
                    )::BIGINT AS avg_duration_ms,
                    COALESCE(
                        percentile_cont(0.95) WITHIN GROUP (ORDER BY r.duration_ms)
                        FILTER (
                            WHERE ast.name IN ('completed', 'warning', 'failed')
                            AND r.duration_ms > 0
                        ),
                        0
                    )::BIGINT AS p95_duration_ms,
                    COALESCE(SUM(r.cost_usd), 0) AS total_cost_usd
                FROM ironflow.runs r
                JOIN lib_fsm.state_machine sm
                    ON sm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state ast
                    ON ast.abstract_state__id = sm.abstract_state__id
                {where_clause}
                GROUP BY bucket_time
                ORDER BY bucket_time ASC
                "#
            );

            let rows = bind_run_filter_params(sqlx::query(&sql), &run_filter)
                .bind(start)
                .bind(now)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let count = |row: &PgRow, column: &str| -> u64 { row.get::<i64, _>(column) as u64 };

            let buckets = rows
                .iter()
                .map(|row| StatsHistoryBucket {
                    time: row.get("bucket_time"),
                    completed: count(row, "completed"),
                    warning: count(row, "warning"),
                    failed: count(row, "failed"),
                    cancelled: count(row, "cancelled"),
                    pending: count(row, "pending"),
                    running: count(row, "running"),
                    retrying: count(row, "retrying"),
                    awaiting_approval: count(row, "awaiting_approval"),
                    sleeping: count(row, "sleeping"),
                    avg_duration_ms: count(row, "avg_duration_ms"),
                    p95_duration_ms: count(row, "p95_duration_ms"),
                    total_cost_usd: row.get::<Decimal, _>("total_cost_usd"),
                })
                .collect();

            Ok(buckets)
        })
    }
}
