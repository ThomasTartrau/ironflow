//! `update_secret` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Update an existing secret's value.
#[mcp_tool(
    name = "update_secret",
    description = "Update the value of an existing secret by key. The response never includes the secret value. Requires admin permissions."
)]
#[derive(serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct UpdateSecretTool {
    /// Secret key to update (e.g. `workflows/inbox/gmail_token`).
    pub key: String,
    /// New secret value (plaintext, will be encrypted at rest).
    pub value: String,
}

impl std::fmt::Debug for UpdateSecretTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateSecretTool")
            .field("key", &self.key)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

impl UpdateSecretTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        super::validate_secret_key(&self.key)?;

        let path = format!("/secrets/{}", self.key);
        let body = json!({ "value": self.value });

        let secret: Value = client.put(&path, &body).await.map_err(CallToolError::new)?;

        let text = to_string_pretty(&secret).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
