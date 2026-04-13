//! `get_workflow` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Get detailed information about a specific workflow.
#[mcp_tool(
    name = "get_workflow",
    description = "Get detailed information about a workflow: description, source code, and sub-workflows."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetWorkflowTool {
    /// The workflow name to look up.
    pub name: String,
}

impl GetWorkflowTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/workflows/{}", self.name);
        let detail: Value = client.get(&path).await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&detail).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
