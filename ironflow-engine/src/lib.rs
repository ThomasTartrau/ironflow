//! # ironflow-engine
//!
//! Workflow orchestration engine for **ironflow**. Supports two workflow styles:
//!
//! - **Static** ([`WorkflowDef`](workflow::WorkflowDef)): serializable step sequences, no chaining.
//! - **Dynamic** ([`WorkflowHandler`](handler::WorkflowHandler)): Rust-native handlers where
//!   steps receive a [`WorkflowContext`](context::WorkflowContext) and can chain outputs.
//!
//! Both can run inline or be enqueued for a background worker.
//!
//! ## Custom operations
//!
//! Implement [`Operation`](operation::Operation) to define custom step types
//! (e.g. GitLab, Gmail, Slack) that integrate into the workflow lifecycle.
//! Call [`WorkflowContext::operation()`](context::WorkflowContext::operation)
//! inside a handler to execute them with full step tracking.
//!
//! # Dynamic workflow example
//!
//! ```no_run
//! use ironflow_engine::prelude::*;
//! use std::future::Future;
//! use std::pin::Pin;
//!
//! struct DeployWorkflow;
//!
//! impl WorkflowHandler for DeployWorkflow {
//!     fn name(&self) -> &str { "deploy" }
//!     fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
//!         Box::pin(async move {
//!             let build = ctx.shell("build", ShellConfig::new("cargo build")).await?;
//!             ctx.agent("review", AgentStepConfig::new(
//!                 &format!("Review: {}", build.output["stdout"])
//!             )).await?;
//!             Ok(())
//!         })
//!     }
//! }
//! ```

pub mod config;
pub mod context;
pub mod engine;
pub mod error;
pub mod executor;
pub mod fsm;
pub mod handler;
pub mod operation;
pub mod workflow;

/// Convenience re-exports.
pub mod prelude {
    pub use crate::config::{AgentStepConfig, HttpConfig, ShellConfig, StepConfig};
    pub use crate::context::WorkflowContext;
    pub use crate::engine::Engine;
    pub use crate::error::EngineError;
    pub use crate::fsm::{RunEvent, RunFsm, StepEvent, StepFsm};
    pub use crate::handler::{HandlerFuture, WorkflowHandler};
    pub use crate::operation::Operation;
    pub use crate::workflow::{StepDef, Workflow, WorkflowDef};
}
