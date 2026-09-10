//! `delete_secret` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::client::ApiClient;

/// Delete a secret by key.
#[mcp_tool(
    name = "delete_secret",
    description = "Delete a secret by its key. Returns a confirmation message on success. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct DeleteSecretTool {
    /// Secret key to delete (e.g. `workflows/inbox/gmail_token`).
    pub key: String,
}

impl DeleteSecretTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        super::validate_secret_key(&self.key)?;

        let path = format!("/secrets/{}", self.key);
        client.delete(&path).await.map_err(CallToolError::new)?;

        let text = format!("Secret '{}' deleted successfully.", self.key);
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
