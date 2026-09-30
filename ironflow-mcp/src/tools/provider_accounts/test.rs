//! `test_provider_account` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, to_string_pretty};

use crate::client::ApiClient;

/// Check the stored token of a Provider Account.
#[mcp_tool(
    name = "test_provider_account",
    description = "Check the stored token of a Provider Account against the provider. Returns result valid, limited or unauthorized and the reported usage windows. The response never includes the token. Requires admin permissions."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct TestProviderAccountTool {
    /// Account name or UUID.
    pub account: String,
}

impl TestProviderAccountTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        super::validate_account_ref(&self.account)?;
        let path = format!("/provider-accounts/{}/test", self.account);
        let body: Value = client
            .post_action(&path)
            .await
            .map_err(CallToolError::new)?;
        let text = to_string_pretty(&body).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
