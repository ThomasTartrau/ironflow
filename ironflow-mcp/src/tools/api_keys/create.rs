//! `create_api_key` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Create a new API key with specified scopes.
#[mcp_tool(
    name = "create_api_key",
    description = "Create a new API key with a name, scopes, and optional expiration. The full raw key is only returned once at creation time. Available scopes can be queried with the Ironflow API."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CreateApiKeyTool {
    /// Human-readable name for the key.
    pub name: String,
    /// Scopes to grant (e.g. ["runs:read", "runs:write", "workflows:read"]).
    pub scopes: Vec<String>,
    /// Optional expiration date in ISO 8601 format.
    pub expires_at: Option<String>,
    /// Optional per-key rate limit override (requests per minute).
    pub rate_limit_override: Option<u32>,
}

impl CreateApiKeyTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut body = json!({
            "name": self.name,
            "scopes": self.scopes,
        });
        if let Some(ref expires_at) = self.expires_at {
            body["expires_at"] = json!(expires_at);
        }
        if let Some(rate_limit) = self.rate_limit_override {
            body["rate_limit_override"] = json!(rate_limit);
        }

        let key: Value = client
            .post("/api-keys", &body)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&key).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
