//! `update_user_role` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Update a user's admin role.
#[mcp_tool(
    name = "update_user_role",
    description = "Update a user's admin role by their ID (UUID). Cannot change your own role. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct UpdateUserRoleTool {
    /// The user ID (UUID) to update.
    pub user_id: String,
    /// New admin status.
    pub is_admin: bool,
}

impl UpdateUserRoleTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/users/{}/role", self.user_id);
        let body = json!({ "is_admin": self.is_admin });

        let user: Value = client
            .patch(&path, &body)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&user).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
