//! Grafana Tempo operations for Ironflow workflows.
//!
//! This crate provides a comprehensive set of Tempo operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation wraps an HTTP call to the Tempo API, using the shared
//! [`reqwest::Client`] from [`OperationContext`](ironflow_core::operation::OperationContext).
//!
//! # Architecture
//!
//! - [`TempoClient`] is the central handle, wrapping a base URL and
//!   authentication credentials
//! - Each operation is a standalone struct implementing
//!   [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "tempo"`
//! - Parameters are set at construction time via builder-style methods
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_tempo::TempoClient;
//! use ironflow_ops_tempo::traces::GetTrace;
//! use ironflow_ops_tempo::status::GetReady;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let tempo = TempoClient::from_context(&ctx).await?;
//!
//! // Retrieve a trace by ID
//! let trace = GetTrace::new(tempo.clone(), "abc123def456");
//! let result = trace.execute(&ctx).await?;
//!
//! // Check readiness
//! let ready = GetReady::new(tempo);
//! let result = ready.execute(&ctx).await?;
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
//! Operations are organized by Tempo API domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`traces`] | GetTrace, GetTraceV2, SearchTraces |
//! | [`tags`] | GetSearchTags, GetSearchTagsV2, GetSearchTagValues, GetSearchTagValuesV2 |
//! | [`metrics`] | QueryMetricsRange, QueryMetricsInstant |
//! | [`overrides`] | GetOverrides, CreateOverrides, UpdateOverrides, DeleteOverrides |
//! | [`status`] | GetReady, GetMetrics, GetBuildInfo, GetStatus, GetVersion, GetServices, GetEndpoints, GetConfig |
//! | [`cluster`] | GetMemberlist, GetDistributorRing, GetLiveStoreRing, GetPartitionRing |
//! | [`maintenance`] | PreparePartitionDownscale, CancelPartitionDownscale, GetPartitionDownscaleStatus, PrepareLiveStoreDownscale, CancelLiveStoreDownscale, GetLiveStoreDownscaleStatus |

mod client;
pub(crate) mod error;

pub mod cluster;
pub mod maintenance;
pub mod metrics;
pub mod overrides;
pub mod status;
pub mod tags;
pub mod traces;

pub use client::TempoClient;
