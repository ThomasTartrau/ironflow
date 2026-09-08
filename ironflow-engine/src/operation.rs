//! [`Operation`] trait re-exported from [`ironflow_core::operation`].
//!
//! The canonical definitions live in `ironflow-core` so that external crates
//! can implement [`Operation`] without depending on `ironflow-engine`.
//! This module re-exports them for backward compatibility.
//!
//! # Examples
//!
//! ```no_run
//! use async_trait::async_trait;
//! use ironflow_engine::operation::Operation;
//! use ironflow_core::operation::OperationContext;
//! use ironflow_core::error::OperationError;
//! use serde_json::{Value, json};
//!
//! struct CreateGitlabIssue {
//!     project_id: u64,
//!     title: String,
//! }
//!
//! #[async_trait]
//! impl Operation for CreateGitlabIssue {
//!     fn kind(&self) -> &str {
//!         "gitlab"
//!     }
//!
//!     async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
//!         Ok(json!({"issue_id": 42, "url": "https://gitlab.com/issues/42"}))
//!     }
//! }
//! ```

pub use ironflow_core::operation::{
    NoopSecretResolver, Operation, OperationContext, SecretResolver, SecretValue, TypedOperation,
};
