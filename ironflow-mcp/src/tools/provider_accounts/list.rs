//! `list_provider_accounts` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, to_string_pretty};

use crate::client::ApiClient;

/// List Provider Accounts.
#[mcp_tool(
    name = "list_provider_accounts",
    description = "List Provider Accounts (AI provider accounts such as Claude subscriptions) with their state and latest usage windows (five_hour, seven_day). The response never includes the token. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListProviderAccountsTool {}

impl ListProviderAccountsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let accounts: Value = client
            .get("/provider-accounts?per_page=100")
            .await
            .map_err(CallToolError::new)?;
        let text = to_string_pretty(&accounts).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
