//! `list_users` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// List all users.
#[mcp_tool(
    name = "list_users",
    description = "List all users in the Ironflow server. Returns ID, email, username, admin status, and timestamps. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListUsersTool {}

impl ListUsersTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let users: Vec<Value> = client.get("/users").await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&users).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
