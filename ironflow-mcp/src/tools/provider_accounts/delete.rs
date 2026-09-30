//! `delete_provider_account` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::client::ApiClient;

/// Delete a Provider Account.
#[mcp_tool(
    name = "delete_provider_account",
    description = "Delete a Provider Account and its stored token. Its usage history is deleted with it. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct DeleteProviderAccountTool {
    /// Account name or UUID.
    pub account: String,
}

impl DeleteProviderAccountTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        super::validate_account_ref(&self.account)?;
        let path = format!("/provider-accounts/{}", self.account);
        client.delete(&path).await.map_err(CallToolError::new)?;
        let text = format!("Provider account '{}' deleted successfully.", self.account);
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
