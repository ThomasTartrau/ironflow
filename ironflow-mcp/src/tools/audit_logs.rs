//! `list_audit_logs` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::to_string_pretty;

use crate::client::ApiClient;

/// List audit log entries with optional filtering.
#[mcp_tool(
    name = "list_audit_logs",
    description = "List audit log entries with optional filters. Returns timestamped domain events for compliance review and debugging. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListAuditLogsTool {
    /// Filter by event type (e.g. `run_status_changed`, `secret_created`).
    pub event_type: Option<String>,
    /// Filter by run ID (UUID).
    pub run_id: Option<String>,
    /// Filter entries created at or after this timestamp (ISO 8601).
    pub from: Option<String>,
    /// Filter entries created at or before this timestamp (ISO 8601).
    pub to: Option<String>,
    /// Page number (1-based, default: 1).
    pub page: Option<u32>,
    /// Items per page (default: 50, max: 100).
    pub per_page: Option<u32>,
}

impl ListAuditLogsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref event_type) = self.event_type {
            query.push(("event_type", event_type.clone()));
        }
        if let Some(ref run_id) = self.run_id {
            query.push(("run_id", run_id.clone()));
        }
        if let Some(ref from) = self.from {
            query.push(("from", from.clone()));
        }
        if let Some(ref to) = self.to {
            query.push(("to", to.clone()));
        }
        if let Some(page) = self.page {
            query.push(("page", page.to_string()));
        }
        if let Some(per_page) = self.per_page {
            query.push(("per_page", per_page.to_string()));
        }

        let params: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let result = client
            .get_raw_with_query("/audit-logs", &params)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
