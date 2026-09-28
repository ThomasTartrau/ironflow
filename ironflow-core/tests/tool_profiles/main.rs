//! Integration tests for per-step tool profiles.
//!
//! [`HttpAgentProvider`](ironflow_core::providers::http::HttpAgentProvider)
//! runs for real against a local TCP server that plays an OpenAI-compatible
//! chat completions API (see [`harness`]).

mod claude_cli;
mod config;
mod harness;
mod http_provider;
