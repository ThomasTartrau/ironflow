//! `plan_workflow` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Build the execution plan of a workflow without running it.
#[mcp_tool(
    name = "plan_workflow",
    description = "Build the execution plan of a workflow without running it: steps, order, dependencies, conditions and parallel groups."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct PlanWorkflowTool {
    /// The workflow name to plan.
    pub name: String,
    /// Optional JSON input payload, as a JSON string, used to evaluate conditions.
    pub payload: Option<String>,
    /// Optional sub-workflow expansion depth (default 3, at most 10).
    pub max_depth: Option<u32>,
}

impl PlanWorkflowTool {
    /// Execute the tool against the Ironflow API.
    ///
    /// # Errors
    ///
    /// Returns [`CallToolError`] when the workflow is unknown, the depth is out
    /// of range, or the API is unreachable.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let parsed_payload: Value = match &self.payload {
            Some(s) => serde_json::from_str(s).unwrap_or(Value::Object(Default::default())),
            None => json!({}),
        };
        let mut body = json!({
            "payload": parsed_payload,
        });
        if let Some(max_depth) = self.max_depth {
            body["max_depth"] = json!(max_depth);
        }

        let path = format!("/workflows/{}/plan", self.name);
        let plan: Value = client
            .post(&path, &body)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&plan).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
