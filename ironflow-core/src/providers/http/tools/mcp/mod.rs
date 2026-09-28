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
mod filter;
pub(crate) mod protocol;

use std::sync::Arc;

use tracing::debug;

pub use bridge::McpBridgeTool;
pub use connection::McpConnection;
pub use error::McpError;
pub use filter::McpToolFilter;

use protocol::McpToolDef;

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

    registry = register_tool_defs(registry, &conn, prefix, tools);
    registry = registry.register_connector(prefix);

    Ok(registry)
}

/// Connect to an MCP server and register only the tools that pass `filter`.
///
/// Unlike [`register_mcp_tools`], which registers every tool the server
/// exposes, this denies everything unless `filter` explicitly permits it: an
/// unconfigured [`McpToolFilter`] (no [`McpToolFilter::allow`], no
/// [`McpToolFilter::require_read_only_hint`]) registers zero tools.
///
/// The number of tools discovered but excluded by `filter` is logged at
/// `debug` level, along with their names.
///
/// # Errors
///
/// Returns [`McpError`] if initialization or tool discovery fails, or
/// [`McpError::ToolNotFound`] if a name passed to
/// [`McpToolFilter::allow`] is not exposed by the server.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::http::tools::ToolRegistry;
/// use ironflow_core::providers::http::tools::mcp::{
///     McpConnection, McpToolFilter, register_mcp_tools_filtered,
/// };
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let conn = McpConnection::http_with_headers(
///     "https://mcp.example.com/mcp",
///     &[("Authorization", "Bearer sk-example")],
/// ).await?;
///
/// let filter = McpToolFilter::new()
///     .allow(&["list_incidents", "get_incident"])
///     .require_read_only_hint();
///
/// let registry = ToolRegistry::new();
/// let registry = register_mcp_tools_filtered(registry, conn, "sentry", filter).await?;
/// # Ok(())
/// # }
/// ```
pub async fn register_mcp_tools_filtered(
    mut registry: ToolRegistry,
    mut connection: McpConnection,
    prefix: &str,
    filter: McpToolFilter,
) -> Result<ToolRegistry, McpError> {
    connection.initialize().await?;
    let tools = connection.list_tools().await?;

    if let Some(allowed) = filter.allowed_names() {
        for name in allowed {
            if !tools.iter().any(|t| &t.name == name) {
                return Err(McpError::ToolNotFound { name: name.clone() });
            }
        }
    }

    let (kept, ignored): (Vec<McpToolDef>, Vec<McpToolDef>) =
        tools.into_iter().partition(|t| filter.matches(t));

    if !ignored.is_empty() {
        debug!(
            prefix = prefix,
            ignored_count = ignored.len(),
            ignored_names = ?ignored.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            "MCP tool filter ignored tools"
        );
    }

    let conn = Arc::new(connection);

    debug!(
        prefix = prefix,
        tool_count = kept.len(),
        "Registering MCP tools (filtered)"
    );

    registry = register_tool_defs(registry, &conn, prefix, kept);
    registry = registry.register_connector(prefix);

    Ok(registry)
}

fn register_tool_defs(
    mut registry: ToolRegistry,
    conn: &Arc<McpConnection>,
    prefix: &str,
    tools: Vec<McpToolDef>,
) -> ToolRegistry {
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
    registry
}
