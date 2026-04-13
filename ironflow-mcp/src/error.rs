//! Error types for the MCP server.

use rust_mcp_sdk::schema::schema_utils::CallToolError;
use thiserror::Error;

/// Errors that can occur during MCP tool execution.
#[derive(Error, Debug)]
pub enum McpError {
    /// HTTP transport error.
    #[error("requete HTTP echouee : {0}")]
    Http(#[from] reqwest::Error),

    /// Ironflow API returned an error response.
    #[error("erreur API ({status}) : {message}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Error message from the API.
        message: String,
    },

    /// JSON deserialization failed.
    #[error("deserialization echouee : {0}")]
    Deserialize(String),
}

impl From<McpError> for CallToolError {
    fn from(err: McpError) -> Self {
        CallToolError::new(err)
    }
}
