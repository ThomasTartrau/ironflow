//! `resume_schedule` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Resume a paused schedule.
#[mcp_tool(
    name = "resume_schedule",
    description = "Resume a paused workflow schedule, re-enabling its automatic triggers."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ResumeScheduleTool {
    /// Schedule UUID.
    pub id: String,
}

impl ResumeScheduleTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/schedules/{}/resume", self.id);
        let schedule: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&schedule).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
