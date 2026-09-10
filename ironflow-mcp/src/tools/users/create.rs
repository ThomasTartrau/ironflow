//! `create_user` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Create a new user account.
#[mcp_tool(
    name = "create_user",
    description = "Create a new user account. Returns the created user (without password). Requires admin permissions."
)]
#[derive(serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CreateUserTool {
    /// Email address.
    pub email: String,
    /// Display username (min 3 characters).
    pub username: String,
    /// Plaintext password (min 8 characters).
    pub password: String,
    /// Whether the new user should be an admin.
    pub is_admin: bool,
}

impl std::fmt::Debug for CreateUserTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateUserTool")
            .field("email", &self.email)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("is_admin", &self.is_admin)
            .finish()
    }
}

impl CreateUserTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let body = json!({
            "email": self.email,
            "username": self.username,
            "password": self.password,
            "is_admin": self.is_admin,
        });

        let user: Value = client
            .post("/users", &body)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&user).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
