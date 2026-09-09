//! Grafana Loki operations for Ironflow workflows.
//!
//! This crate provides a comprehensive set of Loki operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation wraps an HTTP call to the Loki API, using the shared
//! [`reqwest::Client`] from [`OperationContext`](ironflow_core::operation::OperationContext).
//!
//! # Architecture
//!
//! - [`LokiClient`] is the central handle, wrapping a base URL and
//!   authentication credentials
//! - Each operation is a standalone struct implementing
//!   [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "loki"`
//! - Parameters are set at construction time via builder-style methods
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_loki::LokiClient;
//! use ironflow_ops_loki::query::QueryRange;
//! use ironflow_ops_loki::labels::GetLabels;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let loki = LokiClient::from_context(&ctx).await?;
//!
//! // Query logs over a time range
//! let query = QueryRange::new(
//!     loki.clone(),
//!     r#"{job="varlogs"}"#,
//!     "2024-01-01T00:00:00Z",
//!     "2024-01-02T00:00:00Z",
//! );
//! let result = query.execute(&ctx).await?;
//!
//! // List available labels
//! let labels = GetLabels::new(loki);
//! let result = labels.execute(&ctx).await?;
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
//! # Streaming operations
//!
//! [`TailLogs`](query::TailLogs) is a WebSocket streaming operation that does
//! **not** implement `Operation`, following the same pattern as Kubernetes
//! watch/exec operations in `ironflow_ops_k8s`. Access it directly via the
//! Loki WebSocket tail endpoint.
//!
//! # Modules
//!
//! Operations are organized by Loki API domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`query`] | QueryInstant, QueryRange, TailLogs (streaming) |
//! | [`labels`] | GetLabels, GetLabelValues, GetSeries |
//! | [`ingest`] | PushLogs, PushLogsOtlp |
//! | [`index`] | GetIndexStats, GetIndexVolume, GetIndexVolumeRange |
//! | [`patterns`] | DetectPatterns, DetectFields, GetDetectedFieldValues |
//! | [`rules`] | GetRules, GetRulesByNamespace, GetRuleGroup, CreateRuleGroup, DeleteRuleGroup, DeleteRuleNamespace, GetAlerts |
//! | [`delete`] | CreateDeleteRequest, ListDeleteRequests, CancelDeleteRequest |
//! | [`mod@format`] | FormatQuery |
//! | [`status`] | GetReady, GetLogLevel, SetLogLevel, GetMetrics, GetConfig |
//! | [`rings`] | GetDistributorRing, GetIndexGatewayRing, GetRulerRing, GetCompactorRing |
//! | [`ingester`] | Flush, PrepareShutdown, CancelShutdown, Shutdown |

mod client;
pub(crate) mod error;

pub mod delete;
pub mod format;
pub mod index;
pub mod ingest;
pub mod ingester;
pub mod labels;
pub mod patterns;
pub mod query;
pub mod rings;
pub mod rules;
pub mod status;

pub use client::LokiClient;
