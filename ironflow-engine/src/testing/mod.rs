//! In-memory test harness for workflow handlers.
//!
//! [`TestEngine`] runs a real [`Engine`](crate::engine::Engine) against a real
//! [`InMemoryStore`](ironflow_store::memory::InMemoryStore): the run, the steps,
//! the FSM transitions and the persistence are the production ones. Only the
//! outside world is swapped out -- shell commands, HTTP requests, agent
//! invocations and approval gates are answered from closures instead of
//! spawning processes, opening sockets or waiting for a human.
//!
//! What it does *not* start: no HTTP server, no background worker, no Postgres.
//! A run executes inline, in the calling task, and finishes before
//! [`run`](TestEngine::run) returns.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_engine::prelude::*;
//! use ironflow_engine::testing::{ApprovalOutcome, MockShellOutput, TestEngine};
//! use ironflow_store::models::RunStatus;
//! use serde_json::json;
//!
//! # struct Deploy;
//! # impl WorkflowHandler for Deploy {
//! #     fn name(&self) -> &str { "deploy" }
//! #     fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
//! #         Box::pin(async move { ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?; Ok(()) })
//! #     }
//! # }
//! # async fn example() -> Result<(), EngineError> {
//! let result = TestEngine::new()
//!     .with_handler(Deploy)
//!     .with_mock_shell(|_cfg| Ok(MockShellOutput::ok(r#"{"version":"1.2.3"}"#)))
//!     .with_mock_approval(ApprovalOutcome::Approved)
//!     .run(json!({"env": "prod"}))
//!     .await?;
//!
//! assert_eq!(result.status(), RunStatus::Completed);
//! assert_eq!(result.step("deploy").step_output().stdout(), r#"{"version":"1.2.3"}"#);
//! # Ok(())
//! # }
//! ```
//!
//! # What the harness covers
//!
//! | Step | How it is mocked |
//! |------|------------------|
//! | `ctx.shell` | [`TestEngine::with_mock_shell`] |
//! | `ctx.http` | [`TestEngine::with_mock_http`] |
//! | `ctx.agent` | [`TestEngine::with_mock_agent`] or [`TestEngine::with_recorded_agent`] |
//! | `ctx.approval` | [`TestEngine::with_mock_approval`], or [`TestEngine::resume`] |
//! | `ctx.parallel`, `ctx.workflow`, `on_error` | the mocks above apply to the steps inside them |
//!
//! # Limitations
//!
//! * Custom operations ([`ctx.operation`](crate::context::WorkflowContext::operation))
//!   are not intercepted. Mock one by passing a test-double
//!   [`Operation`](crate::operation::Operation) to the handler.
//! * [`ctx.delay`](crate::context::WorkflowContext::delay) is not intercepted: a
//!   non-zero delay still suspends the run with
//!   [`RunStatus::Sleeping`](ironflow_store::models::RunStatus::Sleeping).
//! * [`ctx.decision`](crate::context::WorkflowContext::decision) needs a real
//!   [`DecisionProvider`](ironflow_core::decision::DecisionProvider), wired with
//!   [`TestEngine::with_decision_provider`].

mod engine;
mod mocks;
mod result;

pub use engine::TestEngine;
pub use mocks::{
    AgentMock, HttpMock, MissingAgentProvider, MockAgentProvider, MockHttpResponse,
    MockInterceptor, MockShellOutput, ShellMock,
};
pub use result::{TestResult, TestStep};

// Re-exported so test code has a single import path for everything the harness
// needs.
pub use crate::executor::ApprovalOutcome;
