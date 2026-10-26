//! Integration tests for registry tools combined with structured output.
//!
//! [`HttpAgentProvider`](ironflow_core::providers::http::HttpAgentProvider)
//! runs for real against a local TCP server that plays the Anthropic Messages
//! API or the Gemini `generateContent` API (see [`server`]).

#[cfg(any(feature = "provider-anthropic-api", feature = "provider-gemini"))]
mod server;

#[cfg(feature = "provider-anthropic-api")]
mod anthropic;
#[cfg(feature = "provider-gemini")]
mod gemini;
