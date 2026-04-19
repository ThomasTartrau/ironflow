//! `list_workflows` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde::{Deserialize, Serialize};

use crate::client::ApiClient;

/// Summary returned by `GET /api/v1/workflows`.
#[derive(Debug, Deserialize, Serialize)]
pub struct WorkflowSummary {
    /// Workflow name.
    pub name: String,
    /// Optional `/`-separated category path.
    pub category: Option<String>,
}

/// List all available workflows in Ironflow.
#[mcp_tool(
    name = "list_workflows",
    description = "List all available workflows registered in Ironflow. Returns workflow name + optional /-separated category path."
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListWorkflowsTool {}

impl ListWorkflowsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let workflows: Vec<WorkflowSummary> =
            client.get("/workflows").await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&workflows).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
