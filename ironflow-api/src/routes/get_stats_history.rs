//! `GET /api/v1/stats/history` -- Time-bucketed historical statistics.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::StatsHistoryFilter;

use crate::entities::{StatsHistoryBucketResponse, StatsHistoryQuery, StatsHistoryResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get time-bucketed historical statistics for trend charts.
///
/// Returns aggregated run counts, duration metrics, and cost per time
/// bucket. Accepts optional `workflow`, `period`, and `granularity`
/// query parameters.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/stats/history",
        tags = ["stats"],
        params(StatsHistoryQuery),
        responses(
            (status = 200, description = "Historical statistics", body = StatsHistoryResponse),
            (status = 400, description = "Invalid period or granularity"),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_stats_history(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<StatsHistoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let period = params.period.unwrap_or_default();
    let granularity = params
        .granularity
        .unwrap_or_else(|| period.default_granularity());

    let filter = StatsHistoryFilter {
        workflow_name: params.workflow.clone(),
        period,
        granularity,
    };

    let buckets = state.store.get_stats_history(filter).await?;

    let response = StatsHistoryResponse {
        period,
        granularity,
        workflow: params.workflow,
        buckets: buckets
            .into_iter()
            .map(|b| StatsHistoryBucketResponse {
                time: b.time,
                completed: b.completed,
                failed: b.failed,
                cancelled: b.cancelled,
                avg_duration_ms: b.avg_duration_ms,
                p95_duration_ms: b.p95_duration_ms,
                total_cost_usd: b.total_cost_usd,
            })
            .collect(),
    };

    Ok(ok(response))
}
