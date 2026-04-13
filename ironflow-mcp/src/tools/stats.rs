//! `get_stats` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Get aggregated statistics about workflow executions.
#[mcp_tool(
    name = "get_stats",
    description = "Get aggregated statistics about all workflow executions: total runs, completed, failed, active, success rate, total cost, and total duration."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetStatsTool {}

impl GetStatsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let stats: Value = client.get("/stats").await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&stats).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
