//! `resume_run` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Resume a paused run and the sub-workflow runs paused below it.
#[mcp_tool(
    name = "resume_run",
    description = "Resume a paused workflow run together with the sub-workflow runs paused below it (admin only). The run returns to the state it was paused from; a run paused while it executed goes back to pending and replays from the step where it stopped. The result is the resumed run plus `resumed_descendants`. Fails for a run that is not paused or a sub-workflow run."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ResumeRunTool {
    /// The run ID (UUID) to resume.
    pub run_id: String,
}

impl ResumeRunTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/runs/{}/resume", self.run_id);
        let result: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
