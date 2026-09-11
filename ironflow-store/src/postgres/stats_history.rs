//! PostgreSQL implementation of [`RunStore::get_stats_history`].

use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use sqlx::Row;

use crate::entities::{StatsHistoryBucket, StatsHistoryFilter};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::PostgresStore;

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

            let (wf_clause, has_wf) = if filter.workflow_name.is_some() {
                ("AND r.workflow_name = $3", true)
            } else {
                ("", false)
            };

            let sql = format!(
                r#"
                SELECT
                    date_trunc('{interval}', r.created_at) AS bucket_time,
                    COUNT(*) FILTER (
                        WHERE ast.name IN ('completed', 'warning')
                    ) AS completed,
                    COUNT(*) FILTER (WHERE ast.name = 'failed') AS failed,
                    COUNT(*) FILTER (WHERE ast.name = 'cancelled') AS cancelled,
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
                WHERE r.created_at >= $1
                    AND r.created_at < $2
                    {wf_clause}
                GROUP BY bucket_time
                ORDER BY bucket_time ASC
                "#
            );

            let mut query = sqlx::query(&sql).bind(start).bind(now);
            if has_wf {
                query = query.bind(filter.workflow_name.unwrap());
            }

            let rows = query
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let buckets = rows
                .iter()
                .map(|row| StatsHistoryBucket {
                    time: row.get("bucket_time"),
                    completed: row.get::<i64, _>("completed") as u64,
                    failed: row.get::<i64, _>("failed") as u64,
                    cancelled: row.get::<i64, _>("cancelled") as u64,
                    avg_duration_ms: row.get::<i64, _>("avg_duration_ms") as u64,
                    p95_duration_ms: row.get::<i64, _>("p95_duration_ms") as u64,
                    total_cost_usd: row.get::<Decimal, _>("total_cost_usd"),
                })
                .collect();

            Ok(buckets)
        })
    }
}
