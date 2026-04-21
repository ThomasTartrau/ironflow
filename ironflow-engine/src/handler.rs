//! [`WorkflowHandler`] trait — dynamic workflows with context chaining.
//!
//! Implement this trait to define workflows where steps can reference
//! outputs from previous steps. The handler receives a [`WorkflowContext`]
//! that provides step execution methods with automatic persistence.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_engine::handler::WorkflowHandler;
//! use ironflow_engine::context::WorkflowContext;
//! use ironflow_engine::config::{ShellConfig, AgentStepConfig};
//! use ironflow_engine::error::EngineError;
//! use std::future::Future;
//! use std::pin::Pin;
//!
//! struct DeployWorkflow;
//!
//! impl WorkflowHandler for DeployWorkflow {
//!     fn name(&self) -> &str {
//!         "deploy"
//!     }
//!
//!     fn execute<'a>(
//!         &'a self,
//!         ctx: &'a mut WorkflowContext,
//!     ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
//!         Box::pin(async move {
//!             let build = ctx.shell("build", ShellConfig::new("cargo build --release")).await?;
//!             let tests = ctx.shell("test", ShellConfig::new("cargo test")).await?;
//!
//!             let review = ctx.agent("review", AgentStepConfig::new(
//!                 &format!("Build:\n{}\nTests:\n{}\nReview.",
//!                     build.output["stdout"], tests.output["stdout"])
//!             )).await?;
//!
//!             if review.output["value"].as_str().unwrap_or("").contains("LGTM") {
//!                 ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?;
//!             }
//!
//!             Ok(())
//!         })
//!     }
//! }
//! ```

use std::future::Future;
use std::pin::Pin;

use serde::Serialize;

use crate::context::WorkflowContext;
use crate::error::EngineError;

/// Boxed future returned by [`WorkflowHandler::execute`].
pub type HandlerFuture<'a> = Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>>;

/// Metadata about a workflow, returned by [`WorkflowHandler::describe`].
///
/// Contains a human-readable description and optional Rust source code
/// for display in the dashboard.
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowInfo {
    /// Human-readable description of what the workflow does.
    pub description: String,
    /// Optional Rust source code of the handler (for UI display).
    pub source_code: Option<String>,
    /// Names of sub-workflows invoked by this handler.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_workflows: Vec<String>,
    /// Optional `/`-separated category path used to group workflows in the UI tree.
    ///
    /// A value like `"data/etl"` places the workflow under `data` → `etl`.
    /// `None` means the workflow is uncategorized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Handler version string, used to trace which code produced a given run.
    pub version: String,
}

/// A dynamic workflow handler with context-aware step chaining.
///
/// Implement this trait to define workflows where each step can use
/// the output of previous steps. Register handlers with
/// [`Engine::register`](crate::engine::Engine::register) and execute
/// them by name.
///
/// # Why `Pin<Box<dyn Future>>` instead of `async fn`?
///
/// The handler must be object-safe (`dyn WorkflowHandler`) to allow
/// registering different handler types in the engine's registry.
pub trait WorkflowHandler: Send + Sync {
    /// The workflow name used for registration and lookup.
    fn name(&self) -> &str;

    /// Handler version string, used to trace which code version produced a run.
    ///
    /// Override this to return a meaningful version (semver, git SHA, build
    /// hash, etc.). The default is `"unversioned"`.
    fn version(&self) -> &str {
        "unversioned"
    }

    /// Optional `/`-separated category path used to group workflows in the UI tree.
    ///
    /// Return a value like `"data/etl"` to place the workflow under `data` → `etl`.
    /// The default is `None` (uncategorized).
    ///
    /// Validation (empty segments, leading or trailing `/`, `//`, whitespace
    /// segments) is enforced at registration time by
    /// [`Engine::register`](crate::engine::Engine::register).
    fn category(&self) -> Option<&str> {
        None
    }

    /// Return metadata about this workflow (description, source code).
    ///
    /// Override this to provide a description and source code for the
    /// dashboard UI. The default returns an empty description with no source
    /// but propagates [`WorkflowHandler::category`] and
    /// [`WorkflowHandler::version`].
    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: String::new(),
            source_code: None,
            sub_workflows: Vec::new(),
            category: self.category().map(str::to_string),
            version: self.version().to_string(),
        }
    }

    /// Execute the workflow with the given context.
    ///
    /// The context provides [`shell`](WorkflowContext::shell),
    /// [`http`](WorkflowContext::http), and [`agent`](WorkflowContext::agent)
    /// methods that automatically persist each step.
    ///
    /// # Errors
    ///
    /// Return [`EngineError`] if any step fails. The engine will mark
    /// the run as `Failed` and record the error.
    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a>;
}
