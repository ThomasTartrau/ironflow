//! # ironflow-cli
//!
//! Library crate exposing CLI internals for integration testing.
//!
//! The binary entry point is in `main.rs`; this module re-exports
//! the command handlers and output helpers so that integration tests
//! can invoke them directly against a real server.

pub mod commands;
pub mod config;
pub mod output;
