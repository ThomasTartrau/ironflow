//! `create_run` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

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
    /// How many times to replay the run automatically after a transient failure.
    /// Defaults to 0 (no automatic retry). Each retry waits an exponential
    /// backoff (30s, 2min, 8min, capped at 15min).
    pub max_retries: Option<u32>,
    /// Optional idempotency key making the call safe to replay. Reusing the same
    /// key returns the run it already created instead of starting a second one.
    /// Valid for 24 hours. At most 255 printable ASCII characters.
    pub idempotency_key: Option<String>,
    /// Optional maximum cumulative cost for this run, in USD. Must be zero or
    /// positive. Overrides the workflow and server defaults; omit to use them.
    pub max_cost_usd: Option<f64>,
}

impl CreateRunTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let parsed_payload: Value = match &self.payload {
            Some(s) => serde_json::from_str(s).unwrap_or(Value::Object(Default::default())),
            None => json!({}),
        };
        let mut body = json!({
            "workflow": self.workflow,
            "payload": parsed_payload,
            "max_retries": self.max_retries.unwrap_or(0),
        });
        if let Some(max_cost_usd) = self.max_cost_usd {
            body["max_cost_usd"] = json!(max_cost_usd);
        }

        let run: Value = match &self.idempotency_key {
            Some(key) => client.post_idempotent("/runs", &body, key).await,
            None => client.post("/runs", &body).await,
        }
        .map_err(CallToolError::new)?;

        let text = to_string_pretty(&run).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
