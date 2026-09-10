//! `delete_schedule` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::client::ApiClient;

/// Delete a schedule by ID.
#[mcp_tool(
    name = "delete_schedule",
    description = "Delete a workflow schedule by its UUID. Existing runs created by the schedule are not affected."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct DeleteScheduleTool {
    /// Schedule UUID.
    pub id: String,
}

impl DeleteScheduleTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/schedules/{}", self.id);
        client.delete(&path).await.map_err(CallToolError::new)?;

        let text = format!("Schedule '{}' deleted successfully.", self.id);
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
