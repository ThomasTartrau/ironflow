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
use ironflow_core::error::OperationError;

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct GitLabIssueOp {
        project_id: u64,
        title: String,
    }

    impl Operation for GitLabIssueOp {
        fn kind(&self) -> &str {
            "gitlab"
        }

        fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
            Box::pin(async move {
                Ok(json!({
                    "issue_id": 42,
                    "url": "https://gitlab.com/issues/42",
                    "project_id": self.project_id,
                    "title": self.title
                }))
            })
        }

        fn input(&self) -> Option<Value> {
            Some(json!({
                "project_id": self.project_id,
                "title": self.title
            }))
        }
    }

    struct NoInputOp;

    impl Operation for NoInputOp {
        fn kind(&self) -> &str {
            "noop"
        }

        fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
            Box::pin(async { Ok(json!({"status": "ok"})) })
        }
    }

    struct ErrorOp;

    impl Operation for ErrorOp {
        fn kind(&self) -> &str {
            "error_test"
        }

        fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
            Box::pin(async {
                Err(EngineError::Operation(
                    OperationError::Http {
                        status: Some(500),
                        message: "test error".to_string(),
                    }
                ))
            })
        }
    }

    #[test]
    fn operation_kind_identifies_operation_type() {
        let op = GitLabIssueOp {
            project_id: 123,
            title: "Bug".to_string(),
        };
        assert_eq!(op.kind(), "gitlab");
    }

    #[test]
    fn operation_with_input_provides_structured_logging() {
        let op = GitLabIssueOp {
            project_id: 456,
            title: "Feature request".to_string(),
        };
        let input = op.input();
        assert!(input.is_some());

        let input_value = input.unwrap();
        assert_eq!(input_value["project_id"], 456);
        assert_eq!(input_value["title"], "Feature request");
    }

    #[test]
    fn operation_without_input_returns_none() {
        let op = NoInputOp;
        assert_eq!(op.input(), None);
    }

    #[tokio::test]
    async fn operation_execute_returns_json_output() {
        let op = GitLabIssueOp {
            project_id: 789,
            title: "Test".to_string(),
        };
        let result = op.execute().await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert_eq!(output["issue_id"], 42);
        assert_eq!(output["project_id"], 789);
        assert_eq!(output["title"], "Test");
    }

    #[tokio::test]
    async fn operation_execute_can_return_error() {
        let op = ErrorOp;
        let result = op.execute().await;
        assert!(result.is_err());
    }

    #[test]
    fn operation_kind_identifies_different_types() {
        let gitlab = GitLabIssueOp {
            project_id: 1,
            title: "a".to_string(),
        };
        let noop = NoInputOp;
        let error = ErrorOp;

        assert_eq!(gitlab.kind(), "gitlab");
        assert_eq!(noop.kind(), "noop");
        assert_eq!(error.kind(), "error_test");
    }

    #[tokio::test]
    async fn no_input_operation_executes_successfully() {
        let op = NoInputOp;
        let result = op.execute().await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert_eq!(output["status"], "ok");
    }

    #[test]
    fn gitlab_operation_input_has_all_fields() {
        let op = GitLabIssueOp {
            project_id: 999,
            title: "Complete feature".to_string(),
        };
        let input = op.input().expect("input present");

        assert!(input.is_object());
        assert!(input.get("project_id").is_some());
        assert!(input.get("title").is_some());
    }
}
