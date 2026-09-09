//! Administration operations: health checks, size queries, connection
//! monitoring, and query management.
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`health`] | [`HealthCheck`](health::HealthCheck) |
//! | [`size`] | [`DatabaseSize`](size::DatabaseSize), [`TableSize`](size::TableSize) |
//! | [`connections`] | [`ActiveConnections`](connections::ActiveConnections), [`RunningQueries`](connections::RunningQueries) |
//! | [`process`] | [`CancelQuery`](process::CancelQuery), [`TerminateBackend`](process::TerminateBackend) |

pub mod connections;
pub mod health;
pub mod process;
pub mod size;
