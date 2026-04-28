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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_api_error() {
        let err = McpError::Api {
            status: 404,
            message: "introuvable".to_string(),
        };
        assert_eq!(err.to_string(), "erreur API (404) : introuvable");
    }

    #[test]
    fn display_deserialize_error() {
        let err = McpError::Deserialize("champ manquant".to_string());
        assert_eq!(err.to_string(), "deserialization echouee : champ manquant");
    }

    #[tokio::test]
    async fn display_http_error() {
        let inner = reqwest::Client::new()
            .get("http://127.0.0.1:1/unreachable")
            .send()
            .await
            .unwrap_err();
        let err = McpError::Http(inner);
        let display = err.to_string();
        assert!(display.starts_with("requete HTTP echouee : "));
    }

    #[test]
    fn into_call_tool_error() {
        let err = McpError::Api {
            status: 500,
            message: "boom".to_string(),
        };
        let _tool_err: CallToolError = err.into();
    }
}
