//! Step-kind implementations for [`WorkflowContext`](crate::context::WorkflowContext).
//!
//! One module per step kind. Each holds an `impl WorkflowContext` block with the
//! methods that create, execute and persist that kind of step. They are
//! descendants of `context`, so they read its private fields directly.

mod agent;
mod approval;
mod decision;
mod delay;
mod http;
mod operation;
mod parallel;
mod shell;
mod skip;
mod sub_workflow;
