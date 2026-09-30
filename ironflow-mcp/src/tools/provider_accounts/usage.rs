//! `provider_account_usage` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, to_string_pretty};

use crate::client::ApiClient;

/// Get the usage of a Provider Account.
#[mcp_tool(
    name = "provider_account_usage",
    description = "Get the current usage windows of a Provider Account and their 30-day history. The response never includes the token. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ProviderAccountUsageTool {
    /// Account name or UUID.
    pub account: String,
}

impl ProviderAccountUsageTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        super::validate_account_ref(&self.account)?;
        let path = format!("/provider-accounts/{}/usage", self.account);
        let body: Value = client.get(&path).await.map_err(CallToolError::new)?;
        let text = to_string_pretty(&body).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
