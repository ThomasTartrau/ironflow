//! `reject_input` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Reject a human input step of a run.
#[mcp_tool(
    name = "reject_input",
    description = "Reject a human input step of a workflow execution that is waiting for input. The run resumes and the workflow handler receives the rejection with the reason."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RejectInputTool {
    /// The run ID (UUID) waiting for input.
    pub run_id: String,
    /// The human input step ID (UUID).
    pub step_id: String,
    /// Optional reason passed to the workflow handler.
    pub reason: Option<String>,
}

impl RejectInputTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/runs/{}/steps/{}/reject", self.run_id, self.step_id);
        let body = json!({ "reason": self.reason });
        let result: Value = client
            .post(&path, &body)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
