//! `create_approval_delegation` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json};

use crate::client::ApiClient;

/// Delegate the caller's approval power to another user.
#[mcp_tool(
    name = "create_approval_delegation",
    description = "Delegate your approval power to another user for a time window, so approval gates assigned to you keep moving while you are away. Optionally restrict it to workflows matching a glob such as 'deploy-*'."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CreateApprovalDelegationTool {
    /// UUID of the user receiving the delegated approval power.
    pub to_user_id: String,
    /// End of the window (exclusive), as an RFC 3339 timestamp.
    pub valid_until: String,
    /// Start of the window, as an RFC 3339 timestamp. Defaults to now.
    #[serde(default)]
    pub valid_from: Option<String>,
    /// Glob restricting the delegation to matching workflow names.
    #[serde(default)]
    pub workflow_filter: Option<String>,
}

impl CreateApprovalDelegationTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut body = json!({
            "to_user_id": self.to_user_id,
            "valid_until": self.valid_until,
        });
        // Omitted rather than sent as null: the API defaults `valid_from` to now
        // and reads a missing `workflow_filter` as "every workflow".
        if let Some(valid_from) = &self.valid_from {
            body["valid_from"] = json!(valid_from);
        }
        if let Some(workflow_filter) = &self.workflow_filter {
            body["workflow_filter"] = json!(workflow_filter);
        }

        let delegation: Value = client
            .post("/approval-delegations", &body)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&delegation).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
