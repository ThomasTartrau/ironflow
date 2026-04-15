//! Statistics DTOs.

use rust_decimal::Decimal;
use serde::Serialize;

/// Aggregate statistics response.
///
/// Computed from all runs in the store.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::StatsResponse;
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    /// Total number of runs.
    pub total_runs: u64,
    /// Number of completed runs.
    pub completed_runs: u64,
    /// Number of failed runs.
    pub failed_runs: u64,
    /// Number of cancelled runs.
    pub cancelled_runs: u64,
    /// Number of pending or running runs.
    pub active_runs: u64,
    /// Success rate: completed / (completed + failed), as a percentage.
    pub success_rate_percent: f64,
    /// Aggregated cost across all runs in USD.
    pub total_cost_usd: Decimal,
    /// Aggregated duration across all runs in milliseconds.
    pub total_duration_ms: u64,
}
