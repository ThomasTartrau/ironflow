//! `get_run_logs` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Retrieve persisted log lines for a workflow run.
#[mcp_tool(
    name = "get_run_logs",
    description = "Retrieve persisted log output for a workflow run. Returns log lines with step name, stream (stdout/stderr/system), and content. Supports filtering by step_id and stream, and cursor-based pagination."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetRunLogsTool {
    /// The run ID (UUID).
    pub run_id: String,
    /// Filter by step ID (UUID). Optional.
    pub step_id: Option<String>,
    /// Filter by output stream: stdout, stderr, or system. Optional.
    pub stream: Option<String>,
    /// Maximum number of entries to return (default 100, max 1000). Optional.
    pub limit: Option<u32>,
    /// Cursor from a previous response for pagination. Optional.
    pub cursor: Option<String>,
}

impl GetRunLogsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut params = Vec::new();
        if let Some(ref step_id) = self.step_id {
            params.push(("step_id", step_id.as_str()));
        }
        if let Some(ref stream) = self.stream {
            params.push(("stream", stream.as_str()));
        }
        let limit_str = self.limit.map(|l| l.to_string());
        if let Some(ref s) = limit_str {
            params.push(("limit", s.as_str()));
        }
        if let Some(ref cursor) = self.cursor {
            params.push(("cursor", cursor.as_str()));
        }

        let path = format!("/runs/{}/logs", self.run_id);
        let logs: Value = client
            .get_raw_with_query(&path, &params)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&logs).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
