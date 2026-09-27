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
//!
//! # Missing endpoints
//!
//! Some GitLab REST routes are not implemented by the `gitlab` crate. This crate fills
//! those gaps under [`endpoints`], wired the same way as any other endpoint:
//!
//! ```no_run
//! use ironflow_ops_gitlab::GitLab;
//! use ironflow_ops_gitlab::endpoints::merge_requests::CreateMergeRequestDiscussionNote;
//! use gitlab::api::common::NameOrId;
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let gitlab = GitLab::from_context(&ctx).await?;
//!
//! let endpoint = CreateMergeRequestDiscussionNote {
//!     project: NameOrId::from(42),
//!     merge_request: 7,
//!     discussion_id: "abcd1234".to_string(),
//!     body: "Looks good, thanks!".to_string(),
//! };
//! let op = gitlab.op(endpoint);
//! # Ok(())
//! # }
//! ```
//!
//! # Paginated operations
//!
//! Use [`GitLab::paged_op`] to drive a pageable "list ..." endpoint across every page and
//! get back a single JSON array concatenating every page's results:
//!
//! ```no_run
//! use ironflow_ops_gitlab::GitLab;
//! use gitlab::api::projects::merge_requests::MergeRequests;
//! use gitlab::api::Pagination;
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let gitlab = GitLab::from_context(&ctx).await?;
//!
//! let endpoint = MergeRequests::builder().project(42).build()?;
//! let op = gitlab.paged_op(endpoint, Pagination::All);
//! // op.execute(..) returns a Value::Array concatenating every page
//! # Ok(())
//! # }
//! ```

mod client;
pub mod endpoints;
mod operation;
mod paged_operation;

pub use client::GitLab;
pub use gitlab;
pub use operation::GitLabOp;
pub use paged_operation::GitLabPagedOp;
