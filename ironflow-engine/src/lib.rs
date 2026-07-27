//! # ironflow-engine
//!
//! Workflow orchestration engine for **ironflow**.
//!
//! Workflows are defined as Rust-native handlers implementing
//! [`WorkflowHandler`](handler::WorkflowHandler). Handlers receive a
//! [`WorkflowContext`](context::WorkflowContext) and can chain step outputs,
//! use native `if`/`else`/`match` for conditional branching, and execute
//! steps in parallel.
//!
//! Handlers can be executed inline or enqueued for a background worker.
//!
//! ## Custom operations
//!
//! Implement [`Operation`](operation::Operation) to define custom step types
//! (e.g. GitLab, Gmail, Slack) that integrate into the workflow lifecycle.
//! Call [`WorkflowContext::operation()`](context::WorkflowContext::operation)
//! inside a handler to execute them with full step tracking.
//!
//! # Example
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

pub mod budget;
pub mod config;
pub mod context;
pub mod engine;
pub mod error;
pub mod executor;
pub mod fsm;
pub mod handler;
pub mod log_sender;
pub mod notify;
pub mod operation;
pub mod schedule;

/// Convenience re-exports.
pub mod prelude {
    pub use crate::budget::BudgetConfig;
    pub use crate::config::{AgentStepConfig, ApprovalConfig, HttpConfig, ShellConfig, StepConfig};
    pub use crate::context::WorkflowContext;
    pub use crate::engine::{Engine, EnqueueOptions};
    pub use crate::error::EngineError;
    pub use crate::fsm::{RunEvent, RunFsm, StepEvent, StepFsm};
    pub use crate::handler::{HandlerFuture, WorkflowHandler};
    pub use crate::notify::{Event, EventPublisher, EventSubscriber, WebhookSubscriber};
    pub use crate::operation::Operation;
    pub use crate::schedule::CronSchedule;
}
