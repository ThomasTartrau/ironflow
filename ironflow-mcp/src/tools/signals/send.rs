//! `send_signal` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, from_str, json, to_string_pretty};

use crate::client::ApiClient;

/// Send a signal that resumes the runs waiting for it.
#[mcp_tool(
    name = "send_signal",
    description = "Send a signal to Ironflow. Every run waiting (ctx.wait_for_signal) for the same signal name and key, whose payload schema the payload matches, is resumed. The key identifies one occurrence of the event, such as a commit SHA. Requires an admin, or an API key with the signals_send scope. Reusing an idempotency_id delivers nothing and returns duplicate: true."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SendSignalTool {
    /// Signal name, e.g. "ci.pipeline_finished".
    pub name: String,
    /// Occurrence key, e.g. a commit SHA.
    pub key: String,
    /// The payload as a JSON object string. Defaults to "{}".
    #[serde(default)]
    pub payload: Option<String>,
    /// Deduplication ID: a signal sent again with the same ID is not delivered.
    #[serde(default)]
    pub idempotency_id: Option<String>,
}

impl SendSignalTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        // An unparsable payload is refused here rather than replaced by `{}`.
        let payload: Value = match &self.payload {
            Some(raw) => from_str(raw).map_err(CallToolError::new)?,
            None => json!({}),
        };
        let mut body = json!({
            "name": self.name,
            "key": self.key,
            "payload": payload,
        });
        if let Some(idempotency_id) = &self.idempotency_id {
            body["idempotency_id"] = json!(idempotency_id);
        }

        let delivery: Value = client
            .post("/signals", &body)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&delivery).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
