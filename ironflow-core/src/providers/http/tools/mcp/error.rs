//! Error types for MCP bridge operations.

use thiserror::Error;

/// Errors that can occur during MCP server communication.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::http::tools::mcp::McpError;
///
/// let err = McpError::Timeout { tool_name: "query".to_string() };
/// assert!(err.to_string().contains("query"));
/// ```
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum McpError {
    /// Failed to spawn the MCP server subprocess.
    #[error("failed to spawn MCP server '{command}': {reason}")]
    SpawnFailed {
        /// Command that failed to spawn.
        command: String,
        /// Underlying error message.
        reason: String,
    },
    /// Failed to connect to a remote MCP server.
    #[error("failed to connect to MCP server at '{url}': {reason}")]
    ConnectionFailed {
        /// URL that failed to connect.
        url: String,
        /// Underlying error message.
        reason: String,
    },
    /// MCP protocol-level error (malformed messages, unexpected responses).
    #[error("MCP protocol error: {message}")]
    ProtocolError {
        /// Description of the protocol error.
        message: String,
    },
    /// I/O error during communication.
    #[error("MCP I/O error: {message}")]
    IoError {
        /// Description of the I/O error.
        message: String,
    },
    /// Tool call timed out.
    #[error("MCP tool '{tool_name}' timed out")]
    Timeout {
        /// Name of the tool that timed out.
        tool_name: String,
    },
    /// Tool call returned an error from the MCP server.
    #[error("MCP tool '{tool_name}' failed: {message}")]
    ToolCallFailed {
        /// Name of the tool that failed.
        tool_name: String,
        /// Error message from the server.
        message: String,
    },
    /// Names passed to [`McpToolFilter::allow`](super::McpToolFilter::allow)
    /// were not found among the tools the MCP server exposes.
    #[error(
        "MCP tools in the allow filter not found on the server: {}",
        names.join(", ")
    )]
    ToolNotFound {
        /// Every allow-listed name the server does not expose, sorted.
        names: Vec<String>,
    },
    /// Tools passed to [`McpToolFilter::allow`](super::McpToolFilter::allow)
    /// lack the `readOnlyHint: true` annotation required by
    /// [`McpToolFilter::require_read_only_hint`](super::McpToolFilter::require_read_only_hint).
    #[error(
        "MCP tools in the allow filter are not annotated readOnlyHint: true: {}",
        names.join(", ")
    )]
    ToolNotReadOnly {
        /// Every allow-listed tool without the annotation, sorted.
        names: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_spawn_failed() {
        let err = McpError::SpawnFailed {
            command: "mcp-server".to_string(),
            reason: "not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "failed to spawn MCP server 'mcp-server': not found"
        );
    }

    #[test]
    fn display_connection_failed() {
        let err = McpError::ConnectionFailed {
            url: "http://localhost:8080".to_string(),
            reason: "connection refused".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "failed to connect to MCP server at 'http://localhost:8080': connection refused"
        );
    }

    #[test]
    fn display_protocol_error() {
        let err = McpError::ProtocolError {
            message: "invalid JSON".to_string(),
        };
        assert_eq!(err.to_string(), "MCP protocol error: invalid JSON");
    }

    #[test]
    fn display_io_error() {
        let err = McpError::IoError {
            message: "broken pipe".to_string(),
        };
        assert_eq!(err.to_string(), "MCP I/O error: broken pipe");
    }

    #[test]
    fn display_timeout() {
        let err = McpError::Timeout {
            tool_name: "search".to_string(),
        };
        assert_eq!(err.to_string(), "MCP tool 'search' timed out");
    }

    #[test]
    fn display_tool_call_failed() {
        let err = McpError::ToolCallFailed {
            tool_name: "query".to_string(),
            message: "not authorized".to_string(),
        };
        assert_eq!(err.to_string(), "MCP tool 'query' failed: not authorized");
    }

    #[test]
    fn display_tool_not_found() {
        let err = McpError::ToolNotFound {
            names: vec!["delete_all".to_string(), "purge".to_string()],
        };
        assert_eq!(
            err.to_string(),
            "MCP tools in the allow filter not found on the server: delete_all, purge"
        );
    }

    #[test]
    fn display_tool_not_read_only() {
        let err = McpError::ToolNotReadOnly {
            names: vec!["write_config".to_string()],
        };
        assert_eq!(
            err.to_string(),
            "MCP tools in the allow filter are not annotated readOnlyHint: true: write_config"
        );
    }
}
