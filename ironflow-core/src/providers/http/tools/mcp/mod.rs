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

use tracing::{debug, warn};

pub use bridge::McpBridgeTool;
pub use connection::McpConnection;
pub use error::McpError;
pub use filter::McpToolFilter;

use filter::is_read_only;
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

    registry = register_tool_defs(registry, conn, prefix, tools);
    registry = registry.register_connector(prefix);

    Ok(registry)
}

/// Connect to an MCP server and register only the tools that pass `filter`.
///
/// Unlike [`register_mcp_tools`], which registers every tool the server
/// exposes, this registers only what `filter` permits (see [`McpToolFilter`]).
///
/// Tools named with [`McpToolFilter::allow`] are checked against the server
/// before anything is registered: a missing name, or a name without
/// `readOnlyHint: true` when [`McpToolFilter::require_read_only_hint`] is set,
/// is an error rather than a tool silently left out. Tools excluded by the
/// filter are logged at `debug` level with their names; a filter that keeps
/// no tool at all is logged at `warn` level.
///
/// # Errors
///
/// - [`McpError`] if initialization or tool discovery fails.
/// - [`McpError::ToolNotFound`] if names passed to [`McpToolFilter::allow`]
///   are not exposed by the server.
/// - [`McpError::ToolNotReadOnly`] if the filter requires the read-only hint
///   and allowed tools are not annotated `readOnlyHint: true`.
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
/// let filter = McpToolFilter::allow(["list_incidents", "get_incident"])
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
        let find = |name: &String| tools.iter().find(|t| &t.name == name);

        let missing: Vec<String> = allowed
            .iter()
            .filter(|name| find(name).is_none())
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(McpError::ToolNotFound { names: missing });
        }

        if filter.requires_read_only_hint() {
            let not_read_only: Vec<String> = allowed
                .iter()
                .filter(|name| find(name).is_some_and(|t| !is_read_only(t)))
                .cloned()
                .collect();
            if !not_read_only.is_empty() {
                return Err(McpError::ToolNotReadOnly {
                    names: not_read_only,
                });
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

    if kept.is_empty() {
        warn!(prefix = prefix, "MCP tool filter kept no tool");
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
        let read_only = is_read_only(&tool_def);
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
