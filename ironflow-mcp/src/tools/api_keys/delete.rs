//! `delete_api_key` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::client::ApiClient;

/// Delete an API key by ID.
#[mcp_tool(
    name = "delete_api_key",
    description = "Delete an API key by its ID (UUID). Only keys owned by the authenticated user can be deleted."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct DeleteApiKeyTool {
    /// The API key ID (UUID) to delete.
    pub id: String,
}

impl DeleteApiKeyTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/api-keys/{}", self.id);
        client.delete(&path).await.map_err(CallToolError::new)?;

        let text = format!("API key '{}' deleted successfully.", self.id);
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
