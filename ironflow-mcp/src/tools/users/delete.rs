//! `delete_user` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::client::ApiClient;

/// Delete a user account by ID.
#[mcp_tool(
    name = "delete_user",
    description = "Delete a user account by their ID (UUID). Cannot delete your own account. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct DeleteUserTool {
    /// The user ID (UUID) to delete.
    pub user_id: String,
}

impl DeleteUserTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/users/{}", self.user_id);
        client.delete(&path).await.map_err(CallToolError::new)?;

        let text = format!("User '{}' deleted successfully.", self.user_id);
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
