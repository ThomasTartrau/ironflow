//! GitLab integration for Ironflow workflows.
//!
//! This crate provides a thin integration layer between the
//! [`gitlab`](https://crates.io/crates/gitlab) crate and Ironflow's workflow
//! engine. It re-exports the full `gitlab` API so that workflow handlers get
//! typed, builder-based access to every GitLab endpoint without managing
//! authentication or HTTP clients manually.
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_gitlab::GitLab;
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let gitlab = GitLab::from_context(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Typed queries
//!
//! Use the re-exported [`gitlab::api`] builders and [`gitlab::api::AsyncQuery`] to call any
//! endpoint with a typed response:
//!
//! ```no_run
//! use ironflow_ops_gitlab::GitLab;
//! use gitlab::api::{projects, AsyncQuery};
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let gitlab = GitLab::from_context(&ctx).await?;
//!
//! let endpoint = projects::Project::builder().project(42).build()?;
//! let project: serde_json::Value = endpoint.query_async(gitlab.client()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Tracked operations
//!
//! Wrap any endpoint in [`GitLabOp`] to execute it as a tracked workflow step
//! via `WorkflowContext::operation()`:
//!
//! ```no_run
//! use ironflow_ops_gitlab::GitLab;
//! use gitlab::api::projects;
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let gitlab = GitLab::from_context(&ctx).await?;
//!
//! let endpoint = projects::Project::builder().project(42).build()?;
//! let op = gitlab.op(endpoint);
//! // op implements Operation -- pass it to ctx.operation("get-project", &op)
//! # Ok(())
//! # }
//! ```

mod client;
mod operation;

pub use client::GitLab;
pub use gitlab;
pub use operation::GitLabOp;
