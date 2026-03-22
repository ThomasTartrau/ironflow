//! # ironflow-core
//!
//! Core building blocks for the **ironflow** workflow engine. This crate
//! provides composable, async operations that can be chained via plain Rust
//! variables to build headless CI/CD, DevOps, and AI-powered workflows.
//!
//! # Operations
//!
//! | Operation | Description |
//! |-----------|-------------|
//! | [`Shell`](operations::shell::Shell) | Execute a shell command with timeout, env control, and `kill_on_drop`. |
//! | [`Agent`](operations::agent::Agent) | Invoke an AI agent (Claude Code by default) with structured output support. |
//! | [`Http`](operations::http::Http) | Perform HTTP requests via [`reqwest`] with builder-pattern ergonomics. |
//!
//! # Provider trait
//!
//! The [`AgentProvider`](provider::AgentProvider) trait abstracts the AI
//! backend. The built-in [`ClaudeCodeProvider`](providers::claude::ClaudeCodeProvider)
//! shells out to the `claude` CLI; swap it for
//! [`RecordReplayProvider`](providers::record_replay::RecordReplayProvider)
//! in tests for deterministic, zero-cost replay.
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let files = Shell::new("ls -la").await?;
//!
//! let provider = ClaudeCodeProvider::new();
//! let review = Agent::new()
//!     .prompt(&format!("Summarise:\n{}", files.stdout()))
//!     .model(Model::Haiku)
//!     .max_budget_usd(0.10)
//!     .run(&provider)
//!     .await?;
//!
//! println!("{}", review.text());
//! # Ok(())
//! # }
//! ```

pub mod dry_run;
pub mod error;
pub mod metric_names;
pub mod parallel;
pub mod provider;
pub mod providers;
pub mod retry;
pub mod tracker;
pub mod utils;

pub mod operations {
    pub mod agent;
    pub mod http;
    pub mod shell;
}

pub mod prelude {
    pub use crate::dry_run::{DryRunGuard, is_dry_run, set_dry_run};
    pub use crate::error::{AgentError, OperationError};
    pub use crate::operations::agent::{Agent, AgentResult, Model, PermissionMode};
    pub use crate::operations::http::{Http, HttpOutput};
    pub use crate::operations::shell::{Shell, ShellOutput};
    pub use crate::parallel::{try_join_all, try_join_all_limited};
    pub use crate::provider::AgentProvider;
    pub use crate::providers::claude::ClaudeCodeProvider;
    pub use crate::providers::record_replay::RecordReplayProvider;
    pub use crate::retry::RetryPolicy;
    pub use crate::tracker::WorkflowTracker;
    pub use schemars::JsonSchema;
    pub use serde::{Deserialize, Serialize};
}
