//! `rotate_secret_key` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Re-encrypt a batch of secrets towards a new key version.
#[mcp_tool(
    name = "rotate_secret_key",
    description = "Re-encrypt one batch of secrets towards a target key version. Call repeatedly, passing the returned last_id as after_id, until remaining reaches 0. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RotateSecretKeyTool {
    /// Target key version. Defaults to the server's active version.
    pub to_version: Option<i32>,
    /// Number of secrets to process in this batch. Defaults to 100, max 1000.
    pub batch_size: Option<u32>,
    /// Resume after this secret ID (UUID). Omit to start from the beginning.
    pub after_id: Option<String>,
}

impl RotateSecretKeyTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut body = json!({});
        if let Some(v) = self.to_version {
            body["to_version"] = json!(v);
        }
        if let Some(s) = self.batch_size {
            body["batch_size"] = json!(s);
        }
        if let Some(ref id) = self.after_id {
            body["after_id"] = json!(id);
        }

        let result: Value = client
            .post("/secrets/rotate", &body)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
