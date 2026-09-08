//! [`Operation`] trait re-exported from [`ironflow_core::operation`].
//!
//! The canonical definitions live in `ironflow-core` so that external crates
//! can implement [`Operation`] without depending on `ironflow-engine`.
//! This module re-exports them for backward compatibility.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_engine::operation::Operation;
//! use ironflow_core::operation::OperationContext;
//! use ironflow_core::error::OperationError;
//! use serde_json::{Value, json};
//! use std::future::Future;
//! use std::pin::Pin;
//!
//! struct CreateGitlabIssue {
//!     project_id: u64,
//!     title: String,
//! }
//!
//! impl Operation for CreateGitlabIssue {
//!     fn kind(&self) -> &str {
//!         "gitlab"
//!     }
//!
//!     fn execute<'a>(
//!         &'a self,
//!         _ctx: &'a OperationContext,
//!     ) -> Pin<Box<dyn Future<Output = Result<Value, OperationError>> + Send + 'a>> {
//!         Box::pin(async move {
//!             Ok(json!({"issue_id": 42, "url": "https://gitlab.com/issues/42"}))
//!         })
//!     }
//! }
//! ```

pub use ironflow_core::operation::{
    NoopSecretResolver, Operation, OperationContext, SecretResolver, SecretValue, TypedOperation,
};
