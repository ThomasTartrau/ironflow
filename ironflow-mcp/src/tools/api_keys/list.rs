//! `list_api_keys` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// List all API keys for the authenticated user.
#[mcp_tool(
    name = "list_api_keys",
    description = "List all API keys owned by the authenticated user. Returns key ID, name, prefix (masked), scopes, and timestamps. The full key value is never returned."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListApiKeysTool {}

impl ListApiKeysTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let keys: Vec<Value> = client.get("/api-keys").await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&keys).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
