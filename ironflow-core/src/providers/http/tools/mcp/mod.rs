//! MCP (Model Context Protocol) bridge for the tool registry.
//!
//! This module enables HTTP-based LLM providers to call tools exposed by
//! MCP servers. Each MCP tool is bridged as a [`McpBridgeTool`] that
//! implements the [`Tool`](super::Tool) trait and routes execution to the
//! MCP server via JSON-RPC.
//!
//! # Architecture
//!
//! ```text
//! HttpAgentProvider (agentic loop)
//!     |
//!     v
//! ToolRegistry
//!     |-- WebSearchTool
//!     |-- WebFetchTool
//!     |-- McpBridgeTool("grafana.query_dashboards")
//!     |-- McpBridgeTool("grafana.list_alerts")
//! ```
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::providers::http::tools::ToolRegistry;
//! use ironflow_core::providers::http::tools::mcp::{McpConnection, register_mcp_tools};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let conn = McpConnection::stdio(
//!     "mcp-grafana",
//!     &["stdio"],
//!     &[("GRAFANA_URL", "http://grafana:3000")],
//! ).await?;
//!
//! let registry = ToolRegistry::new();
//! let registry = register_mcp_tools(registry, conn, "grafana").await?;
//! # Ok(())
//! # }
//! ```

mod bridge;
mod connection;
mod error;
pub(crate) mod protocol;

use std::sync::Arc;

use tracing::debug;

pub use bridge::McpBridgeTool;
pub use connection::McpConnection;
pub use error::McpError;

use super::ToolRegistry;

/// Connect to an MCP server and register all its tools into a [`ToolRegistry`].
///
/// Each tool name is prefixed with `{prefix}.` to avoid collisions with other
/// tools or MCP servers (e.g., `grafana.query_dashboards`).
///
/// # Errors
///
/// Returns [`McpError`] if initialization or tool discovery fails.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::http::tools::ToolRegistry;
/// use ironflow_core::providers::http::tools::mcp::{McpConnection, register_mcp_tools};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let conn = McpConnection::stdio("mcp-server", &[], &[]).await?;
/// let registry = ToolRegistry::new();
/// let registry = register_mcp_tools(registry, conn, "myserver").await?;
/// # Ok(())
/// # }
/// ```
pub async fn register_mcp_tools(
    mut registry: ToolRegistry,
    mut connection: McpConnection,
    prefix: &str,
) -> Result<ToolRegistry, McpError> {
    connection.initialize().await?;
    let tools = connection.list_tools().await?;
    let conn = Arc::new(connection);

    debug!(
        prefix = prefix,
        tool_count = tools.len(),
        "Registering MCP tools"
    );

    for tool_def in tools {
        let registry_name = format!("{}.{}", prefix, tool_def.name);
        let bridge = McpBridgeTool::new(
            conn.clone(),
            registry_name,
            tool_def.name,
            tool_def.description.unwrap_or_default(),
            tool_def.input_schema,
        );
        registry = registry.register(bridge);
    }

    Ok(registry)
}
