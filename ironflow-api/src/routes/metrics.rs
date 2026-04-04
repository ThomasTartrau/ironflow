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
//! # Grafana example
//!
//! ```promql
//! # Run throughput (completed runs per minute)
//! rate(ironflow_runs_total{status="completed"}[5m])
//!
//! # P99 run duration
//! histogram_quantile(0.99, rate(ironflow_run_duration_seconds_bucket[5m]))
//!
//! # Agent cost rate (USD per hour)
//! rate(ironflow_agent_cost_usd_total[1h])
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
