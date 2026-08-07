//! `retry_run` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Retry a failed workflow execution.
#[mcp_tool(
    name = "retry_run",
    description = "Retry a failed, cancelled, or retrying workflow execution. Creates a new run with the same workflow and payload. Pass force=true to override a handler version mismatch."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RetryRunTool {
    /// The run ID (UUID) to retry.
    pub run_id: String,
    /// Force the retry even when the handler version has changed.
    #[serde(default)]
    pub force: Option<bool>,
}

impl RetryRunTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let force_qs = if self.force.unwrap_or(false) {
            "?force=true"
        } else {
            ""
        };
        let path = format!("/runs/{}/retry{}", self.run_id, force_qs);
        let result: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
