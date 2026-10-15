//! Prometheus metrics endpoint.
//!
//! Exposes all collected metrics in the Prometheus text exposition format.
//! Only available when the `prometheus` feature is enabled.
//!
//! # Endpoint
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/api/v1/metrics` | Returns metrics in Prometheus text format |
//!
//! # Authentication
//!
//! Metrics reveal workflow names, run volumes and agent costs, so the endpoint
//! is not public. It answers an admin session, or an API key holding the
//! `stats_read` (or `admin`) scope whose owner is an admin. An unauthenticated
//! request gets `401`, any other caller `403`.
//!
//! Prometheus scrapes it with such an API key as a bearer token:
//!
//! ```yaml
//! scrape_configs:
//!   - job_name: ironflow
//!     metrics_path: /api/v1/metrics
//!     authorization:
//!       type: Bearer
//!       credentials_file: /etc/prometheus/ironflow-api-key
//!     static_configs:
//!       - targets: ["ironflow:3000"]
//! ```
//!
//! # Metrics exposed
//!
//! All metrics defined in [`ironflow_core::metric_names`] are available,
//! including run counters, step histograms, agent cost/token gauges,
//! worker activity, and API request latency.
//!
//! # Grafana dashboard
//!
//! Recommended layout: three rows (Overview, Steps & Operations, Worker),
//! each with 2-4 panels. All queries assume the default Prometheus data
//! source and a `$job` variable set to the scrape job name.
//!
//! ## Row 1 -- Overview
//!
//! ```promql
//! # Panel: Run throughput (completed runs per minute, by workflow)
//! sum by (workflow) (rate(ironflow_runs_total{status="Completed"}[5m]))
//!
//! # Panel: Run failure rate (percentage)
//! sum(rate(ironflow_runs_total{status="Failed"}[5m]))
//!   / sum(rate(ironflow_runs_total[5m])) * 100
//!
//! # Panel: P50 / P95 / P99 run duration
//! histogram_quantile(0.50, rate(ironflow_run_duration_seconds_bucket[5m]))
//! histogram_quantile(0.95, rate(ironflow_run_duration_seconds_bucket[5m]))
//! histogram_quantile(0.99, rate(ironflow_run_duration_seconds_bucket[5m]))
//!
//! # Panel: Active runs (gauge)
//! ironflow_runs_active
//! ```
//!
//! ## Row 2 -- Steps & Operations
//!
//! ```promql
//! # Panel: Step throughput by kind
//! sum by (kind) (rate(ironflow_steps_total[5m]))
//!
//! # Panel: Agent cost rate (USD per hour, by model)
//! sum by (model) (rate(ironflow_agent_cost_usd_total[1h]))
//!
//! # Panel: Agent token throughput (input + output per minute)
//! rate(ironflow_agent_tokens_input_total[5m])
//! rate(ironflow_agent_tokens_output_total[5m])
//! rate(ironflow_agent_tokens_cache_read_total[5m])
//!
//! # Panel: Shell / HTTP step duration P95
//! histogram_quantile(0.95, rate(ironflow_shell_duration_seconds_bucket[5m]))
//! histogram_quantile(0.95, rate(ironflow_http_duration_seconds_bucket[5m]))
//! ```
//!
//! ## Row 3 -- Worker
//!
//! ```promql
//! # Panel: Queue depth (pending runs waiting for a worker)
//! ironflow_worker_queue_depth
//!
//! # Panel: Runs held back by a saturated concurrency group
//! sum by (group) (ironflow_worker_queue_blocked_runs)
//!
//! # Panel: Active worker tasks
//! ironflow_worker_active
//!
//! # Panel: Poll hit/miss rate
//! rate(ironflow_worker_polls_total{result="hit"}[5m])
//! rate(ironflow_worker_polls_total{result="miss"}[5m])
//!
//! # Panel: Lease losses (counter, alerts if > 0)
//! increase(ironflow_worker_leases_lost_total[1h])
//! ```
//!
//! ## Row 4 -- API
//!
//! ```promql
//! # Panel: API request rate by path
//! sum by (path) (rate(ironflow_api_requests_total[5m]))
//!
//! # Panel: API latency P95 by path
//! histogram_quantile(0.95, sum by (le, path)
//!   (rate(ironflow_api_request_duration_seconds_bucket[5m])))
//! ```
//!
//! ## Alerting rules
//!
//! ```promql
//! # Alert: high run failure rate (> 10% over 15 minutes)
//! sum(rate(ironflow_runs_total{status="Failed"}[15m]))
//!   / sum(rate(ironflow_runs_total[15m])) > 0.10
//!
//! # Alert: queue depth growing (> 10 pending runs for 5 minutes)
//! ironflow_worker_queue_depth > 10
//!
//! # Alert: no worker activity (zero polls for 5 minutes)
//! rate(ironflow_worker_polls_total[5m]) == 0
//!
//! # Alert: budget exceeded (any run cancelled for cost)
//! increase(ironflow_run_budget_exceeded_total[1h]) > 0
//!
//! # Alert: a schedule failed to fire, or was disabled because its next
//! # trigger cannot be computed (`last_error` on the schedule says why)
//! sum by (schedule) (increase(ironflow_schedule_fire_errors_total[15m])) > 0
//!
//! # Alert: a schedule dropped occurrences (downtime past the catch-up window,
//! # catch-up policy, or overlap with a still-active run)
//! sum by (schedule) (increase(ironflow_schedule_missed_total[1h])) > 0
//! ```

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::ApiKeyScope;
use metrics_exporter_prometheus::PrometheusHandle;

use crate::authorization::AuthorizeExt;
use crate::error::ApiError;

/// Handler for `GET /api/v1/metrics`.
///
/// Returns all registered Prometheus metrics in text exposition format, to an
/// admin session or an admin-owned API key holding `stats_read`.
///
/// # Errors
///
/// - 401 if the request is not authenticated
/// - 403 `INSUFFICIENT_SCOPE` if the API key lacks `stats_read`
/// - 403 `FORBIDDEN` if the session user, or the API key owner, is not an admin
///
/// # Examples
///
/// ```no_run
/// use axum::Router;
/// use axum::routing::get;
/// use ironflow_api::routes::metrics::metrics;
/// use ironflow_api::state::AppState;
///
/// let app: Router<AppState> = Router::new().route("/metrics", get(metrics));
/// ```
pub async fn metrics(
    auth: Authenticated,
    State(handle): State<PrometheusHandle>,
) -> Result<Response, ApiError> {
    auth.require(ApiKeyScope::StatsRead, true)?;
    let body = handle.render();
    Ok((
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response())
}
