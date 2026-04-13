//! `list_workflows` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::client::ApiClient;

/// List all available workflows in Ironflow.
#[mcp_tool(
    name = "list_workflows",
    description = "List all available workflows registered in Ironflow. Returns workflow names."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListWorkflowsTool {}

impl ListWorkflowsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let workflows: Vec<String> = client.get("/workflows").await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&workflows).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
