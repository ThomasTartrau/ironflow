//! `list_secrets` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// List all secrets (names and metadata only, never the values).
#[mcp_tool(
    name = "list_secrets",
    description = "List all secrets stored in the Ironflow server. Returns name, ID, and timestamps for each secret. Secret values are never exposed. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListSecretsTool {}

impl ListSecretsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let secrets: Vec<Value> = client.get("/secrets").await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&secrets).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
