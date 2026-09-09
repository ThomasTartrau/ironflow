//! Grafana Mimir operations for Ironflow workflows.
//!
//! This crate provides a comprehensive set of Mimir operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation wraps an HTTP call to the Mimir API (Prometheus-compatible),
//! using the shared [`reqwest::Client`] from
//! [`OperationContext`](ironflow_core::operation::OperationContext).
//!
//! # Architecture
//!
//! - [`MimirClient`] is the central handle, wrapping a base URL and
//!   authentication credentials
//! - Each operation is a standalone struct implementing
//!   [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "mimir"`
//! - Parameters are set at construction time via builder-style methods
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_mimir::MimirClient;
//! use ironflow_ops_mimir::query::QueryRange;
//! use ironflow_ops_mimir::series::GetLabels;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let mimir = MimirClient::from_context(&ctx).await?;
//!
//! // Query metrics over a time range
//! let query = QueryRange::new(
//!     mimir.clone(),
//!     "up",
//!     "2024-01-01T00:00:00Z",
//!     "2024-01-02T00:00:00Z",
//! );
//! let result = query.execute(&ctx).await?;
//!
//! // List available labels
//! let labels = GetLabels::new(mimir);
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
//! # Modules
//!
//! Operations are organized by Mimir API domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`query`] | QueryInstant, QueryRange, QueryExemplars, FormatQuery |
//! | [`series`] | GetSeries, GetLabels, GetLabelValues, GetMetadata, GetActiveSeries |
//! | [`cardinality`] | GetLabelNamesCardinality, GetLabelValuesCardinality |
//! | [`ingest`] | RemoteWrite, OtlpMetricsWrite, InfluxWrite |
//! | [`rules`] | GetRules, GetRulesByNamespace, GetRuleGroup, CreateRuleGroup, DeleteRuleGroup, DeleteRuleNamespace, GetAllTenantRules |
//! | [`alerts`] | GetAlerts, GetAlertmanagerConfig, SetAlertmanagerConfig, DeleteAlertmanagerConfig, GetAlertmanagerStatus, GetAlertmanagerConfigs |
//! | [`distributor`] | GetDistributorRing, GetDistributorUserStats, GetHaTrackerStatus |
//! | [`ingester`] | Flush, PrepareShutdown, CancelShutdown, Shutdown, PreparePartitionDownscale, CancelPartitionDownscale, GetIngesterRing, GetIngesterTenants |
//! | [`store_gateway`] | GetRing, GetTenants, GetTenantBlocks, PrepareShutdown |
//! | [`compactor`] | GetRing, StartBlockUpload, UploadBlockFile, FinishBlockUpload, GetTenants |
//! | [`status`] | GetReady, GetMetrics, GetConfig, GetConfigDiff, GetServices, GetBuildInfo, GetUserLimits |

mod client;
pub(crate) mod error;

pub mod alerts;
pub mod cardinality;
pub mod compactor;
pub mod distributor;
pub mod ingest;
pub mod ingester;
pub mod query;
pub mod rules;
pub mod series;
pub mod status;
pub mod store_gateway;

pub use client::MimirClient;
