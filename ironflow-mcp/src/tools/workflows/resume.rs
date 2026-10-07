//! `resume_workflow` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Resume a paused workflow: workers pick its queued runs again.
#[mcp_tool(
    name = "resume_workflow",
    description = "Resume a paused workflow (admin only): workers pick its queued runs again. Resuming a workflow that is not paused is accepted and changes nothing."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ResumeWorkflowTool {
    /// The workflow name to resume.
    pub name: String,
}

impl ResumeWorkflowTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/workflows/{}/resume", self.name);
        let result: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
