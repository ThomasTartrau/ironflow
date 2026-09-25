//! `replay_run` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, to_string_pretty};

use crate::client::ApiClient;

/// Replay a finished workflow execution on the current handler version.
#[mcp_tool(
    name = "replay_run",
    description = "Replay a finished (completed, failed, warning, or cancelled) workflow execution as a new run with the same payload, on the workflow's current handler version. The original run is not modified."
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReplayRunTool {
    /// The run ID (UUID) to replay.
    pub run_id: String,
}

impl ReplayRunTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/runs/{}/replay", self.run_id);
        let result: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
