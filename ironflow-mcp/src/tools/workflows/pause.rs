//! `pause_workflow` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Pause a workflow: workers stop picking its queued runs.
#[mcp_tool(
    name = "pause_workflow",
    description = "Pause a whole workflow (admin only): workers stop picking its queued runs, new runs are still created and wait in the queue. Runs already executing are left alone; pause them one by one with pause_run. Pausing a workflow already paused is accepted and keeps its first paused_at."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct PauseWorkflowTool {
    /// The workflow name to pause.
    pub name: String,
}

impl PauseWorkflowTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/workflows/{}/pause", self.name);
        let result: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
