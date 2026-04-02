//! [`Operation`] trait — user-defined step operations.
//!
//! Implement this trait to create custom step types that integrate into the
//! workflow lifecycle. Common use cases include API clients (GitLab, Gmail,
//! Slack) that need full step tracking (persistence, duration, error handling).
//!
//! # How it works
//!
//! 1. Implement [`Operation`] on your type.
//! 2. Call [`WorkflowContext::operation()`](crate::context::WorkflowContext::operation)
//!    inside a [`WorkflowHandler`](crate::handler::WorkflowHandler).
//! 3. The engine handles the full step lifecycle: create step record, transition
//!    to Running, execute, persist output/duration, mark Completed or Failed.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_engine::operation::Operation;
//! use ironflow_engine::error::EngineError;
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
//!     fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
//!         Box::pin(async move {
//!             // Call the GitLab API here (e.g. via the `gitlab` crate).
//!             Ok(json!({"issue_id": 42, "url": "https://gitlab.com/issues/42"}))
//!         })
//!     }
//! }
//! ```

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::EngineError;

/// A user-defined operation that integrates into the workflow step lifecycle.
///
/// Implement this trait for custom integrations (GitLab, Gmail, Slack, etc.)
/// that need full step tracking when executed via
/// [`WorkflowContext::operation()`](crate::context::WorkflowContext::operation).
///
/// # Contract
///
/// - [`kind()`](Operation::kind) returns a short, lowercase identifier stored
///   as [`StepKind::Custom`](ironflow_store::entities::StepKind::Custom) in
///   the database (e.g. `"gitlab"`, `"gmail"`, `"slack"`).
/// - [`execute()`](Operation::execute) performs the operation and returns
///   a JSON [`Value`] on success. The engine persists this as the step output.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::operation::Operation;
/// use ironflow_engine::error::EngineError;
/// use serde_json::{Value, json};
/// use std::future::Future;
/// use std::pin::Pin;
///
/// struct SendSlackMessage {
///     channel: String,
///     text: String,
/// }
///
/// impl Operation for SendSlackMessage {
///     fn kind(&self) -> &str { "slack" }
///
///     fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
///         Box::pin(async move {
///             // Post to Slack API ...
///             Ok(json!({"ok": true, "ts": "1234567890.123456"}))
///         })
///     }
/// }
/// ```
pub trait Operation: Send + Sync {
    /// A short, lowercase identifier for this operation type.
    ///
    /// Stored as [`StepKind::Custom(kind)`](ironflow_store::entities::StepKind::Custom)
    /// in the database. Examples: `"gitlab"`, `"gmail"`, `"slack"`.
    fn kind(&self) -> &str;

    /// Execute the operation and return the result as JSON.
    ///
    /// The returned [`Value`] is persisted as the step output. On error,
    /// the engine marks the step as Failed and records the error message.
    ///
    /// # Errors
    ///
    /// Return [`EngineError`] if the operation fails.
    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>>;

    /// Optional JSON representation of the operation input, stored in
    /// the step's `input` column for observability.
    ///
    /// Defaults to [`None`]. Override to provide structured input logging.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use ironflow_engine::operation::Operation;
    /// # use ironflow_engine::error::EngineError;
    /// # use serde_json::{Value, json};
    /// # use std::pin::Pin;
    /// # use std::future::Future;
    /// # struct MyOp;
    /// # impl Operation for MyOp {
    /// #     fn kind(&self) -> &str { "test" }
    /// #     fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
    /// #         Box::pin(async { Ok(json!({})) })
    /// #     }
    /// fn input(&self) -> Option<Value> {
    ///     Some(json!({"project_id": 123, "title": "Bug report"}))
    /// }
    /// # }
    /// ```
    fn input(&self) -> Option<Value> {
        None
    }
}
