//! PostgreSQL operations for Ironflow workflows, powered by [`sqlx`].
//!
//! This crate provides PostgreSQL operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation wraps a `sqlx` query, using a shared [`PgPool`](sqlx::PgPool)
//! for connection management.
//!
//! # Architecture
//!
//! - [`PostgresClient`] is the central handle, wrapping a connection pool
//! - Each operation is a standalone struct implementing [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "postgres"`
//! - Queries use `sqlx::query()` (runtime) because workflows define their
//!   SQL at execution time
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_postgres::PostgresClient;
//! use ironflow_ops_postgres::admin::health::HealthCheck;
//! use ironflow_ops_postgres::query::QueryRows;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let client = PostgresClient::connect("postgres://localhost/mydb").await?;
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//!
//! // Health check
//! let health = HealthCheck::new(client.pool().clone());
//! health.execute(&ctx).await?;
//!
//! // Query rows
//! let query = QueryRows::new(
//!     client.pool().clone(),
//!     "SELECT id, name FROM users WHERE active = $1",
//!     vec![serde_json::json!(true)],
//! );
//! let result = query.execute(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Tracked operations
//!
//! Every operation implements [`Operation`](ironflow_core::operation::Operation),
//! so it can be passed to `WorkflowContext::operation()` for step lifecycle
//! tracking (step record, status transitions, duration, output persistence).
//!
//! # Modules
//!
//! Operations are organized by domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`query`] | QueryRows, QueryOne, QueryScalar |
//! | [`execute`] | Execute, ExecuteBatch, Transaction |
//! | [`schema`] | ListDatabases, ListSchemas, ListTables, ListColumns, ListIndexes, ListConstraints, TableExists |
//! | [`admin`] | HealthCheck, DatabaseSize, TableSize, ActiveConnections, RunningQueries, CancelQuery, TerminateBackend |
//! | [`maintenance`] | Vacuum, Analyze, Reindex |

pub mod admin;
mod client;
pub mod execute;
mod helpers;
pub mod maintenance;
pub mod query;
pub mod schema;

pub use client::PostgresClient;
pub use sqlx;
