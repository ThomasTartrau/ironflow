//! `create_schedule` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json};

use crate::client::ApiClient;

/// Create a new workflow schedule.
#[mcp_tool(
    name = "create_schedule",
    description = "Create a new periodic schedule that triggers a workflow on a cron expression. Returns the created schedule with its ID and next trigger time."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CreateScheduleTool {
    /// Name of the workflow to trigger.
    pub workflow_name: String,
    /// Cron expression (5-field standard or 6-field with seconds).
    pub cron_expression: String,
    /// JSON inputs to pass to the workflow on each trigger (as a JSON string).
    #[serde(default)]
    pub inputs: Option<String>,
}

impl CreateScheduleTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let inputs: Value = match &self.inputs {
            Some(s) => serde_json::from_str(s).map_err(CallToolError::new)?,
            None => json!({}),
        };
        let body = json!({
            "workflow_name": self.workflow_name,
            "cron_expression": self.cron_expression,
            "inputs": inputs,
        });
        let schedule: Value = client
            .post("/schedules", &body)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&schedule).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
