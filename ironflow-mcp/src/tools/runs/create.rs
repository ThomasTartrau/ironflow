//! `create_run` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Trigger a workflow execution.
#[mcp_tool(
    name = "create_run",
    description = "Trigger the execution of a workflow. Returns the created run with its ID and status."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CreateRunTool {
    /// The workflow name to trigger.
    pub workflow: String,
    /// Optional JSON payload to pass to the workflow as a JSON string. Defaults to {}.
    pub payload: Option<String>,
    /// Optional idempotency key making the call safe to replay. Reusing the same
    /// key returns the run it already created instead of starting a second one.
    /// Valid for 24 hours. At most 255 printable ASCII characters.
    pub idempotency_key: Option<String>,
}

impl CreateRunTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let parsed_payload: Value = match &self.payload {
            Some(s) => serde_json::from_str(s).unwrap_or(Value::Object(Default::default())),
            None => serde_json::json!({}),
        };
        let body = serde_json::json!({
            "workflow": self.workflow,
            "payload": parsed_payload,
        });

        let run: Value = match &self.idempotency_key {
            Some(key) => client.post_idempotent("/runs", &body, key).await,
            None => client.post("/runs", &body).await,
        }
        .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&run).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
