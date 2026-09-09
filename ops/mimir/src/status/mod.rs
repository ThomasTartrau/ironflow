//! Instance status and configuration operations.
//!
//! These operations query the running Mimir instance's readiness,
//! metrics, configuration, services, build info, and per-user limits.

mod health;
mod info;

pub use health::{GetMetrics, GetReady, GetServices};
pub use info::{GetBuildInfo, GetConfig, GetConfigDiff, GetUserLimits};
