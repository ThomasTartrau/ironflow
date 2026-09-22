//! `delete_approval_delegation` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::client::ApiClient;

/// Revoke an approval delegation by ID.
#[mcp_tool(
    name = "delete_approval_delegation",
    description = "Revoke an approval delegation by its UUID. Only the delegator who granted it, or an admin, may revoke it."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct DeleteApprovalDelegationTool {
    /// Delegation UUID.
    pub id: String,
}

impl DeleteApprovalDelegationTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let path = format!("/approval-delegations/{}", self.id);
        client.delete(&path).await.map_err(CallToolError::new)?;

        let text = format!("Delegation '{}' revoked successfully.", self.id);
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
