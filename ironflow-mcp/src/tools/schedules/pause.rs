//! `pause_schedule` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Pause a schedule (disable automatic triggers).
#[mcp_tool(
    name = "pause_schedule",
    description = "Pause a workflow schedule, disabling its automatic triggers. The schedule can be resumed later."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct PauseScheduleTool {
    /// Schedule UUID.
    pub id: String,
}

impl PauseScheduleTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/schedules/{}/pause", self.id);
        let schedule: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&schedule).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
