//! Built-in [`AgentProvider`](crate::provider::AgentProvider) implementations.
//!
//! * [`claude::ClaudeCodeProvider`] - production provider that invokes the
//!   `claude` CLI.
//! * [`record_replay::RecordReplayProvider`] - test-friendly wrapper that
//!   records and replays agent responses from JSON fixtures.

pub mod claude;
pub mod record_replay;
