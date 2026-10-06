//! Integration tests for the cancellation of sub-workflow runs.
//!
//! A child run executes inside its parent's execution: once the parent stops,
//! nothing drives it any more. These tests check that no child is left
//! non-terminal (holding its concurrency key) when its parent is cancelled,
//! fails, schedules a retry or is abandoned, and that cancelling a child
//! directly lets its parent observe the cancellation. Every test drives a
//! real [`Engine`](ironflow_engine::engine::Engine) over a real
//! [`InMemoryStore`](ironflow_store::memory::InMemoryStore) and real shell
//! steps.
//!
//! Test names contain `orphan_child` so `cargo test -p ironflow-engine
//! orphan_child` selects them.

mod cancelled_child;
mod fixture;
mod stopped_parent;
