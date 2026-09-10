//! `create_secret` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Create or update a secret.
#[mcp_tool(
    name = "create_secret",
    description = "Create or update a secret. If the key already exists, the value is replaced. The response never includes the secret value. Key format: alphanumeric, '/', '-', '_', '.' (e.g. 'workflows/inbox/gmail_token'). Requires admin permissions."
)]
#[derive(serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CreateSecretTool {
    /// Secret key (namespaced, e.g. `workflows/inbox/gmail_token`).
    pub key: String,
    /// Secret value (plaintext, will be encrypted at rest).
    pub value: String,
}

impl std::fmt::Debug for CreateSecretTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateSecretTool")
            .field("key", &self.key)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

impl CreateSecretTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let body = json!({
            "key": self.key,
            "value": self.value,
        });

        let secret: Value = client
            .post("/secrets", &body)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&secret).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
