//! `get_run` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Get detailed information about a specific run and its steps.
#[mcp_tool(
    name = "get_run",
    description = "Get detailed information about a workflow execution including all its steps, their status, output, duration, and cost."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetRunTool {
    /// The run ID (UUID).
    pub run_id: String,
}

impl GetRunTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/runs/{}", self.run_id);
        let detail: Value = client.get(&path).await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&detail).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
