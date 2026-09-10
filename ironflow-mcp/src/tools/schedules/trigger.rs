//! `trigger_schedule` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Trigger a schedule manually, creating a run immediately.
#[mcp_tool(
    name = "trigger_schedule",
    description = "Manually trigger a workflow schedule, creating a new run immediately with the schedule's configured inputs."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct TriggerScheduleTool {
    /// Schedule UUID.
    pub id: String,
}

impl TriggerScheduleTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/schedules/{}/trigger", self.id);
        let schedule: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&schedule).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
