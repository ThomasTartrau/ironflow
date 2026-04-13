//! `list_runs` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// List workflow executions with optional filters.
#[mcp_tool(
    name = "list_runs",
    description = "List workflow executions with optional filters. Returns a paginated list of runs with status, cost, and duration."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListRunsTool {
    /// Filter by workflow name (exact match).
    pub workflow: Option<String>,
    /// Filter by status: pending, running, completed, failed, retrying, cancelled, awaiting_approval.
    pub status: Option<String>,
    /// Page number (1-based, default: 1).
    pub page: Option<u32>,
    /// Items per page (default: 20, max: 100).
    pub per_page: Option<u32>,
}

impl ListRunsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref w) = self.workflow {
            query.push(("workflow", w.clone()));
        }
        if let Some(ref s) = self.status {
            query.push(("status", s.clone()));
        }
        if let Some(p) = self.page {
            query.push(("page", p.to_string()));
        }
        if let Some(pp) = self.per_page {
            query.push(("per_page", pp.to_string()));
        }

        let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let result: Value = client
            .get_raw_with_query("/runs", &query_refs)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
