//! `pause_run` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Pause a run and the sub-workflow runs below it.
#[mcp_tool(
    name = "pause_run",
    description = "Pause an unfinished workflow run together with every sub-workflow run below it (admin only). A queued, sleeping or waiting run is no longer picked or woken; a running run has its step in flight interrupted, executed again on resume. The run keeps the state it was paused from in `resume_status`. The result is the paused run plus `paused_descendants`, the ids of the sub-runs paused with it. Fails for a finished run, a run already paused, or a sub-workflow run (pause its root run instead)."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct PauseRunTool {
    /// The run ID (UUID) to pause.
    pub run_id: String,
}

impl PauseRunTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/runs/{}/pause", self.run_id);
        let result: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
