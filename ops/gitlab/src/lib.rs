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
//! # Secret masking
//!
//! A tracked operation masks secrets in its request body before sending it,
//! so text produced by an agent or a command never publishes a token in a
//! note, an issue or a merge request. The client's own token is masked, plus
//! the values added with [`GitLab::with_secrets`] and token-shaped strings
//! (`sk-ant-...`, `glpat-...`). [`GitLabOp::masks_secrets`] tells whether
//! anything is masked. A query sent through [`GitLab::client`] is not masked.
//!
//! ```no_run
//! use ironflow_ops_gitlab::GitLab;
//! use gitlab::api::projects::issues::CreateIssue;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let worker_token = String::new();
//! # let agent_summary = String::new();
//! let gitlab = GitLab::new("glpat-xxxx", "gitlab.com")
//!     .await?
//!     .with_secrets([worker_token]);
//!
//! let endpoint = CreateIssue::builder()
//!     .project(42)
//!     .title("Follow-up")
//!     .description(agent_summary)
//!     .build()?;
//! let op = gitlab.op(endpoint);
//! // ctx.operation("create-issue", &op) publishes the masked description
//! # Ok(())
//! # }
//! ```
//!
//! # Missing endpoints
//!
//! Some GitLab REST routes are not implemented by the `gitlab` crate. This crate fills
//! those gaps under [`endpoints`], wired the same way as any other endpoint. Besides merge
//! request replies and versions, it covers issue and commit discussions, single note reads,
//! releases and file uploads (`endpoints::{discussions, notes, releases, uploads}`):
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
mod error;
mod masked;
mod operation;
mod paged_operation;

pub use client::GitLab;
pub use gitlab;
pub use operation::GitLabOp;
pub use paged_operation::GitLabPagedOp;
