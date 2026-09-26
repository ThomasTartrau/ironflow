//! `submit_input` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, from_str, to_string_pretty};

use crate::client::ApiClient;

/// Answer a human input step of a run.
#[mcp_tool(
    name = "submit_input",
    description = "Answer a human input step of a workflow execution that is waiting for input. The value must match the JSON schema stored on the step (see get_run). The run resumes with the answer."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SubmitInputTool {
    /// The run ID (UUID) waiting for input.
    pub run_id: String,
    /// The human input step ID (UUID).
    pub step_id: String,
    /// The answer as a JSON string, matching the step's JSON schema.
    pub value: String,
}

impl SubmitInputTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        // An unparsable answer is refused here rather than replaced by a default.
        let value: Value = from_str(&self.value).map_err(CallToolError::new)?;
        let path = format!("/runs/{}/steps/{}/input", self.run_id, self.step_id);
        let result: Value = client
            .post(&path, &value)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
