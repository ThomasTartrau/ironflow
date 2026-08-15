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
//! ```

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use metrics_exporter_prometheus::PrometheusHandle;

/// Handler for `GET /api/v1/metrics`.
///
/// Returns all registered Prometheus metrics in text exposition format.
///
/// # Examples
///
/// ```no_run
/// use axum::Router;
/// use axum::routing::get;
/// use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
///
/// # fn example() {
/// let handle = PrometheusBuilder::new()
///     .install_recorder()
///     .expect("failed to install recorder");
///
/// let app: Router<PrometheusHandle> = Router::new()
///     .route("/metrics", get(ironflow_api::routes::metrics::metrics));
/// # }
/// ```
pub async fn metrics(State(handle): State<PrometheusHandle>) -> Response {
    let body = handle.render();
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response()
}
