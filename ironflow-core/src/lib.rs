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
//! # Known limitations: Structured output
//!
//! When using [`AgentConfig::output::<T>()`](provider::AgentConfig::output) to request
//! structured (typed) output from the Claude CLI, be aware of these upstream bugs:
//!
//! | Issue | Impact |
//! |-------|--------|
//! | [claude-code#18536] | `structured_output` is always `null` when tools are used alongside `--json-schema`. ironflow prevents this at compile time via typestate (tools and schema are mutually exclusive). |
//! | [claude-code#9058] | The CLI does not validate output against the provided JSON schema -- non-conforming JSON may be returned. |
//! | [claude-agent-sdk-python#502] | Wrapper objects with a single array field may be flattened to a bare array (e.g. `[...]` instead of `{"items": [...]}`). |
//! | [claude-agent-sdk-python#374] | The wrapping behavior is non-deterministic: the same prompt can produce differently shaped output across runs. |
//!
//! **Recommended workarounds:**
//!
//! 1. **Two-step pattern**: use one agent with tools to gather data, then a second
//!    agent with `.output::<T>()` (no tools) to structure the result.
//! 2. **Defensive deserialization**: when deserializing structured output, handle
//!    both the expected wrapper object and a bare array/value as fallback.
//! 3. **`max_turns >= 2`**: structured output requires at least 2 turns; setting
//!    `max_turns(1)` with a schema will fail with `error_max_turns`.
//!
//! [claude-code#18536]: https://github.com/anthropics/claude-code/issues/18536
//! [claude-code#9058]: https://github.com/anthropics/claude-code/issues/9058
//! [claude-agent-sdk-python#502]: https://github.com/anthropics/claude-agent-sdk-python/issues/502
//! [claude-agent-sdk-python#374]: https://github.com/anthropics/claude-agent-sdk-python/issues/374
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
//!     .model(Model::HAIKU)
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
pub mod schema_transform;
#[cfg(test)]
pub(crate) mod test_support;
pub mod tracker;
pub mod utils;

/// Workflow operations (shell commands, agent calls, HTTP requests).
pub mod operations {
    pub mod agent;
    pub mod http;
    pub mod shell;
}

/// Re-exports of the most commonly used types.
pub mod prelude {
    pub use crate::dry_run::{DryRunGuard, is_dry_run, set_dry_run};
    pub use crate::error::{AgentError, OperationError};
    pub use crate::operations::agent::{Agent, AgentResult, Model, PermissionMode};
    pub use crate::operations::http::{Http, HttpOutput};
    pub use crate::operations::shell::{Shell, ShellOutput};
    pub use crate::parallel::{try_join_all, try_join_all_limited};
    pub use crate::provider::{AgentConfig, AgentProvider, DebugMessage, DebugToolCall, LogSink};
    pub use crate::providers::claude::ClaudeCodeProvider;
    pub use crate::providers::record_replay::RecordReplayProvider;
    pub use crate::retry::RetryPolicy;
    pub use crate::tracker::WorkflowTracker;
    pub use schemars::JsonSchema;
    pub use serde::{Deserialize, Serialize};

    #[cfg(feature = "provider-openai")]
    pub use crate::providers::http::OpenAiProvider;
    #[cfg(feature = "provider-mistral")]
    pub use crate::providers::http::MistralProvider;
    #[cfg(feature = "provider-gemini")]
    pub use crate::providers::http::GeminiProvider;
    #[cfg(feature = "provider-anthropic-api")]
    pub use crate::providers::http::AnthropicApiProvider;
}
