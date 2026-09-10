//! `list_schedules` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// List all workflow schedules.
#[mcp_tool(
    name = "list_schedules",
    description = "List all workflow schedules with their cron expressions, status, and next trigger times."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListSchedulesTool {}

impl ListSchedulesTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let schedules: Vec<Value> = client.get("/schedules").await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&schedules).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
