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
//!     |-- McpBridgeTool("grafana__query_dashboards")
//!     |-- McpBridgeTool("grafana__list_alerts")
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
use super::routing::CONNECTOR_SEPARATOR;

/// Connect to an MCP server and register all its tools into a [`ToolRegistry`].
///
/// Each tool name is prefixed with `{prefix}__` (double underscore) to avoid
/// collisions with other tools or MCP servers (e.g., `grafana__query_dashboards`).
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
    registry: ToolRegistry,
    mut connection: McpConnection,
    prefix: &str,
) -> Result<ToolRegistry, McpError> {
    connection.initialize().await?;
    register_shared_mcp_tools(registry, &Arc::new(connection), prefix).await
}

/// Register the tools of an already initialized, shared MCP connection.
///
/// Use it to put one MCP server in several tool profiles
/// ([`HttpAgentProvider::with_tool_profile`](crate::providers::http::HttpAgentProvider::with_tool_profile))
/// without opening it once per profile: every bridged tool holds the same
/// `Arc`. Tool names are prefixed as in [`register_mcp_tools`].
///
/// # Errors
///
/// Returns [`McpError`] if tool discovery fails.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_core::providers::http::tools::ToolRegistry;
/// use ironflow_core::providers::http::tools::mcp::{McpConnection, register_shared_mcp_tools};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut gitlab = McpConnection::stdio("mcp-gitlab", &[], &[]).await?;
/// gitlab.initialize().await?;
/// let gitlab = Arc::new(gitlab);
///
/// let suggestion = register_shared_mcp_tools(ToolRegistry::new(), &gitlab, "gitlab").await?;
/// let bug = register_shared_mcp_tools(ToolRegistry::new(), &gitlab, "gitlab").await?;
/// # Ok(())
/// # }
/// ```
pub async fn register_shared_mcp_tools(
    mut registry: ToolRegistry,
    conn: &Arc<McpConnection>,
    prefix: &str,
) -> Result<ToolRegistry, McpError> {
    let tools = conn.list_tools().await?;

    debug!(
        prefix = prefix,
        tool_count = tools.len(),
        "Registering MCP tools"
    );

    for tool_def in tools {
        let registry_name = format!("{}{CONNECTOR_SEPARATOR}{}", prefix, tool_def.name);
        let read_only = tool_def
            .annotations
            .as_ref()
            .is_some_and(|a| a.read_only_hint);
        let bridge = McpBridgeTool::new(
            conn.clone(),
            registry_name,
            tool_def.name,
            tool_def.description.unwrap_or_default(),
            tool_def.input_schema,
            read_only,
        );
        registry = registry.register(bridge);
    }

    registry = registry.register_connector(prefix);

    Ok(registry)
}
