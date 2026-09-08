//! [`Operation`] trait and [`OperationContext`] for user-defined step operations.
//!
//! This module provides the extension point for custom step types that integrate
//! into the workflow lifecycle. Common use cases include API clients (GitLab,
//! Gmail, Slack) that need full step tracking.
//!
//! # How it works
//!
//! 1. Implement [`Operation`] on your type.
//! 2. Call `WorkflowContext::operation()` inside a `WorkflowHandler`.
//! 3. The engine handles the full step lifecycle: create step record, transition
//!    to Running, execute, persist output/duration, mark Completed or Failed.
//!
//! # OperationContext
//!
//! [`OperationContext`] is passed to every [`Operation::execute`] call. It
//! provides a shared [`reqwest::Client`] and a [`SecretResolver`] so that
//! operations do not need to create their own HTTP clients or manage
//! credentials manually.
//!
//! # Examples
//!
//! ```no_run
//! use async_trait::async_trait;
//! use ironflow_core::operation::{Operation, OperationContext};
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
//!     async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
//!         // Use ctx.http_client() for HTTP calls, ctx.secrets() for credentials.
//!         Ok(json!({"issue_id": 42, "url": "https://gitlab.com/issues/42"}))
//!     }
//! }
//! ```

#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::OperationError;

/// A decrypted secret value returned by [`SecretResolver::get`].
///
/// Wraps a plaintext string. The value is only available after successful
/// decryption by the underlying store.
///
/// # Examples
///
/// ```
/// use ironflow_core::operation::SecretValue;
///
/// let secret = SecretValue { value: "sk-ant-12345".to_string() };
/// assert_eq!(secret.value, "sk-ant-12345");
/// ```
#[derive(Clone)]
pub struct SecretValue {
    /// The decrypted plaintext value.
    pub value: String,
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretValue")
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Trait for resolving secrets at operation execution time.
///
/// Implementations provide read-only access to encrypted secrets scoped
/// to the current workflow. The engine passes a resolver through
/// [`OperationContext`] so that operations can fetch credentials without
/// depending on the store crate.
///
/// # Examples
///
/// ```
/// use ironflow_core::operation::{SecretResolver, NoopSecretResolver};
///
/// # tokio_test::block_on(async {
/// let resolver = NoopSecretResolver;
/// let result = resolver.get("any_key").await;
/// assert!(result.unwrap().is_none());
/// # });
/// ```
#[async_trait]
pub trait SecretResolver: Send + Sync {
    /// Look up a secret by key.
    ///
    /// Returns `Ok(Some(value))` if the secret exists, `Ok(None)` if it
    /// does not, or `Err` on storage/decryption failure.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the underlying store fails.
    async fn get(&self, key: &str) -> Result<Option<SecretValue>, OperationError>;
}

/// A [`SecretResolver`] that always returns `Ok(None)`.
///
/// Used in tests and when the `secret-store` feature is disabled.
///
/// # Examples
///
/// ```
/// use ironflow_core::operation::{SecretResolver, NoopSecretResolver};
///
/// # tokio_test::block_on(async {
/// let resolver = NoopSecretResolver;
/// assert!(resolver.get("anything").await.unwrap().is_none());
/// # });
/// ```
pub struct NoopSecretResolver;

#[async_trait]
impl SecretResolver for NoopSecretResolver {
    async fn get(&self, _key: &str) -> Result<Option<SecretValue>, OperationError> {
        Ok(None)
    }
}

/// Context provided to every [`Operation::execute`] call.
///
/// Carries a shared HTTP client and a secret resolver so that operations
/// do not need to manage their own connections or credentials.
///
/// # Examples
///
/// ```
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let _client = ctx.http_client();
/// ```
pub struct OperationContext {
    http_client: Client,
    secrets: Arc<dyn SecretResolver>,
}

impl OperationContext {
    /// Create a new context with a default HTTP client.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// ```
    pub fn new(secrets: Arc<dyn SecretResolver>) -> Self {
        Self {
            http_client: Client::new(),
            secrets,
        }
    }

    /// Create a new context with a custom HTTP client.
    ///
    /// Use this to share a single [`Client`] across multiple operations
    /// within the same workflow run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use reqwest::Client;
    /// use std::sync::Arc;
    ///
    /// let client = Client::new();
    /// let ctx = OperationContext::with_http_client(client, Arc::new(NoopSecretResolver));
    /// ```
    pub fn with_http_client(http_client: Client, secrets: Arc<dyn SecretResolver>) -> Self {
        Self {
            http_client,
            secrets,
        }
    }

    /// The shared HTTP client for this operation context.
    pub fn http_client(&self) -> &Client {
        &self.http_client
    }

    /// The secret resolver for this operation context.
    pub fn secrets(&self) -> &dyn SecretResolver {
        &*self.secrets
    }
}

/// A user-defined operation that integrates into the workflow step lifecycle.
///
/// Implement this trait for custom integrations (GitLab, Gmail, Slack, etc.)
/// that need full step tracking when executed via `WorkflowContext::operation()`.
///
/// # Contract
///
/// - [`kind()`](Operation::kind) returns a short, lowercase identifier stored
///   as `StepKind::Custom` in the database (e.g. `"gitlab"`, `"gmail"`, `"slack"`).
/// - [`execute()`](Operation::execute) performs the operation and returns
///   a JSON [`Value`] on success. The engine persists this as the step output.
///
/// # Examples
///
/// ```no_run
/// use async_trait::async_trait;
/// use ironflow_core::operation::{Operation, OperationContext};
/// use ironflow_core::error::OperationError;
/// use serde_json::{Value, json};
///
/// struct SendSlackMessage {
///     channel: String,
///     text: String,
/// }
///
/// #[async_trait]
/// impl Operation for SendSlackMessage {
///     fn kind(&self) -> &str { "slack" }
///
///     async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
///         // Post to Slack API using ctx.http_client() ...
///         Ok(json!({"ok": true, "ts": "1234567890.123456"}))
///     }
/// }
/// ```
#[async_trait]
pub trait Operation: Send + Sync {
    /// A short, lowercase identifier for this operation type.
    ///
    /// Stored as `StepKind::Custom(kind)` in the database.
    /// Examples: `"gitlab"`, `"gmail"`, `"slack"`.
    fn kind(&self) -> &str;

    /// Execute the operation and return the result as JSON.
    ///
    /// The returned [`Value`] is persisted as the step output. On error,
    /// the engine marks the step as Failed and records the error message.
    ///
    /// # Errors
    ///
    /// Return [`OperationError`] if the operation fails.
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError>;

    /// Optional JSON representation of the operation input, stored in
    /// the step's `input` column for observability.
    ///
    /// Defaults to [`None`]. Override to provide structured input logging.
    fn input(&self) -> Option<Value> {
        None
    }
}

/// A typed extension of [`Operation`] that declares a concrete output type.
///
/// Implement this alongside [`Operation`] when the step output has a known
/// Rust type. Consumers can then deserialize the output without guessing
/// the shape.
///
/// # Examples
///
/// ```no_run
/// use async_trait::async_trait;
/// use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
/// use ironflow_core::error::OperationError;
/// use serde::Deserialize;
/// use serde_json::{Value, json};
///
/// #[derive(Debug, Deserialize)]
/// struct IssueCreated {
///     iid: u64,
///     url: String,
/// }
///
/// struct CreateIssue;
///
/// #[async_trait]
/// impl Operation for CreateIssue {
///     fn kind(&self) -> &str { "gitlab" }
///     async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
///         Ok(json!({"iid": 42, "url": "https://gitlab.com/issues/42"}))
///     }
/// }
///
/// impl TypedOperation for CreateIssue {
///     type Output = IssueCreated;
/// }
/// ```
pub trait TypedOperation: Operation {
    /// The concrete output type that [`Operation::execute`] produces.
    type Output: DeserializeOwned;
}
