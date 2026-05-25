//! Bridge between MCP tools and the ironflow [`Tool`] trait.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use super::connection::McpConnection;
use super::error::McpError;
use crate::providers::http::tools::{Tool, ToolError, ToolOutput};

/// A bridge that exposes a single MCP tool as an ironflow [`Tool`].
///
/// Each `McpBridgeTool` instance represents one tool from an MCP server.
/// Multiple instances from the same server share a single [`McpConnection`]
/// via `Arc`.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use serde_json::json;
/// use ironflow_core::providers::http::tools::mcp::{McpBridgeTool, McpConnection};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut conn = McpConnection::stdio("mcp-server", &[], &[]).await?;
/// conn.initialize().await?;
///
/// let tool = McpBridgeTool::new(
///     Arc::new(conn),
///     "srv.search".to_string(),
///     "search".to_string(),
///     "Search for documents".to_string(),
///     json!({"type": "object", "properties": {"query": {"type": "string"}}}),
/// );
/// # Ok(())
/// # }
/// ```
pub struct McpBridgeTool {
    connection: Arc<McpConnection>,
    registry_name: String,
    mcp_name: String,
    description: String,
    parameters_schema: Value,
}

impl McpBridgeTool {
    /// Create a new MCP bridge tool.
    ///
    /// `registry_name` is the prefixed name used in the tool registry (e.g. `grafana.echo`).
    /// `mcp_name` is the original tool name as known by the MCP server (e.g. `echo`).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use serde_json::json;
    /// use ironflow_core::providers::http::tools::Tool;
    /// use ironflow_core::providers::http::tools::mcp::{McpBridgeTool, McpConnection};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let conn = Arc::new(McpConnection::stdio("mcp-server", &[], &[]).await?);
    /// let tool = McpBridgeTool::new(
    ///     conn,
    ///     "srv.echo".to_string(),
    ///     "echo".to_string(),
    ///     "Echo tool".to_string(),
    ///     json!({"type": "object", "properties": {}}),
    /// );
    /// assert_eq!(tool.name(), "srv.echo");
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        connection: Arc<McpConnection>,
        registry_name: String,
        mcp_name: String,
        description: String,
        parameters_schema: Value,
    ) -> Self {
        Self {
            connection,
            registry_name,
            mcp_name,
            description,
            parameters_schema,
        }
    }
}

impl Tool for McpBridgeTool {
    fn name(&self) -> &str {
        &self.registry_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters_schema.clone()
    }

    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            match self.connection.call_tool(&self.mcp_name, input).await {
                Ok(content) => Ok(ToolOutput::success(content)),
                Err(McpError::Timeout { tool_name }) => Ok(ToolOutput::error(format!(
                    "MCP tool '{tool_name}' timed out"
                ))),
                Err(McpError::ToolCallFailed { tool_name, message }) => Ok(ToolOutput::error(
                    format!("MCP tool '{tool_name}' error: {message}"),
                )),
                Err(e) => Err(ToolError::new(format!("MCP infrastructure error: {e}"))),
            }
        })
    }
}
