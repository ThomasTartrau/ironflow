//! `list_approval_delegations` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::to_string_pretty;

use crate::client::ApiClient;

/// List the active approval delegations visible to the caller.
#[mcp_tool(
    name = "list_approval_delegations",
    description = "List the active approval delegations visible to the caller, with their delegator, delegate, validity window, and optional workflow filter. Expired delegations are never listed. Paginated: the response meta carries page, per_page and total. The user filters apply to admins only."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListApprovalDelegationsTool {
    /// Only delegations granted by this user ID (admin only).
    pub from_user_id: Option<String>,
    /// Only delegations received by this user ID (admin only).
    pub to_user_id: Option<String>,
    /// Page number (1-based, default: 1).
    pub page: Option<u32>,
    /// Items per page (default: 20, max: 100).
    pub per_page: Option<u32>,
}

impl ListApprovalDelegationsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref from_user_id) = self.from_user_id {
            query.push(("from_user_id", from_user_id.clone()));
        }
        if let Some(ref to_user_id) = self.to_user_id {
            query.push(("to_user_id", to_user_id.clone()));
        }
        if let Some(page) = self.page {
            query.push(("page", page.to_string()));
        }
        if let Some(per_page) = self.per_page {
            query.push(("per_page", per_page.to_string()));
        }

        let params: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let result = client
            .get_raw_with_query("/approval-delegations", &params)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
