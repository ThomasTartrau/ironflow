//! # ironflow-cli
//!
//! Library crate exposing CLI internals for integration testing.
//!
//! The binary entry point is in `main.rs`; this module re-exports
//! the command tree, the command handlers, and the output helpers so that
//! integration tests can invoke them directly against a real server.

pub mod cli;
pub mod commands;
pub mod config;
pub mod confirm;
pub mod output;

pub use cli::{Cli, Commands};
