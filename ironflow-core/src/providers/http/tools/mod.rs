//! Client-side tool execution for HTTP-based LLM providers.
//!
//! This module provides a [`Tool`] trait and [`ToolRegistry`] that enable
//! HTTP providers to execute tools locally and loop back results to the model,
//! turning a single-turn API call into a full agentic loop.
//!
//! # Architecture
//!
//! 1. Tools are registered in a [`ToolRegistry`].
//! 2. The registry converts tools to the OpenAI `tools` array format for the request.
//! 3. When the model responds with `tool_calls`, the provider executes them via the registry.
//! 4. Results are appended to the messages and the model is called again.
//!
//! # Feature Gates
//!
//! Each concrete tool is behind its own feature flag:
//! - `tool-bash` - `BashTool`
//! - `tool-read-file` - `ReadFileTool`
//! - `tool-web-fetch` - `WebFetchTool`
//! - `tool-web-search` - `WebSearchTool`

#[cfg(feature = "tool-bash")]
pub mod bash;
#[cfg(feature = "tool-mcp")]
pub mod mcp;
#[cfg(feature = "tool-read-file")]
pub mod read_file;
#[cfg(feature = "tool-web-fetch")]
pub mod web_fetch;
#[cfg(feature = "tool-web-search")]
pub mod web_search;

mod registry;
mod tool_trait;

pub use registry::ToolRegistry;
pub use tool_trait::{Tool, ToolError, ToolOutput};
