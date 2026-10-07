//! `cancel_run` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Cancel a running workflow execution and the sub-workflow runs below it.
#[mcp_tool(
    name = "cancel_run",
    description = "Cancel an in-progress workflow execution together with every sub-workflow run below it that is still active. Only works on runs that are not yet finished (pending, running, retrying, awaiting_approval, sleeping, paused); cancelling an already cancelled run is accepted. The result is the cancelled run plus `cancelled_descendants`, the ids of the sub-runs cancelled with it. Cancelling a sub-run directly makes its parent fail, unless the parent tolerates the failure."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CancelRunTool {
    /// The run ID (UUID) to cancel.
    pub run_id: String,
}

impl CancelRunTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/runs/{}/cancel", self.run_id);
        let result: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
