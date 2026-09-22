//! `list_approval_delegations` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// List the active approval delegations visible to the caller.
#[mcp_tool(
    name = "list_approval_delegations",
    description = "List the active approval delegations visible to the caller, with their delegator, delegate, validity window, and optional workflow filter. Expired delegations are never listed."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListApprovalDelegationsTool {}

impl ListApprovalDelegationsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let delegations: Vec<Value> = client
            .get("/approval-delegations")
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&delegations).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
