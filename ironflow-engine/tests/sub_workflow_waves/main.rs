//! Integration tests for named, labelled and concurrent sub-workflows.
//!
//! `ctx.workflow_with` names its step and labels its child run through
//! `WorkflowOptions`; `ctx.workflows_with` runs a wave of children of one
//! handler concurrently. Every test drives a real
//! [`Engine`](ironflow_engine::engine::Engine) over a real
//! [`InMemoryStore`](ironflow_store::memory::InMemoryStore); agent steps are
//! answered by a `MockAgentProvider`, or by a provider that waits for the
//! test. `cargo test -p ironflow-engine --test sub_workflow_waves <word>`
//! selects a group.

mod fixture;
mod options;
mod resume;
mod wave;
