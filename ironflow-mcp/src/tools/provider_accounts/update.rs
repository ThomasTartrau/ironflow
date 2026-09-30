//! `update_provider_account` MCP tool.

use std::fmt;

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Map, Value, to_string_pretty};

use crate::client::ApiClient;

/// Update a Provider Account.
#[mcp_tool(
    name = "update_provider_account",
    description = "Update a Provider Account: display name, enabled, priority, tags, max concurrency, alert threshold, plan, or replace its token (checked against the provider first). The name is immutable. The response never includes the token. Requires admin permissions."
)]
#[derive(serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct UpdateProviderAccountTool {
    /// Account name or UUID.
    pub account: String,
    /// New display name.
    pub display_name: Option<String>,
    /// Enable or disable the account.
    pub enabled: Option<bool>,
    /// New priority.
    pub priority: Option<i32>,
    /// New tags (replaces the list).
    pub tags: Option<Vec<String>>,
    /// New maximum concurrent steps.
    pub max_concurrency: Option<u32>,
    /// New alert threshold, in (0, 1].
    pub alert_threshold: Option<f64>,
    /// New plan.
    pub plan: Option<String>,
    /// Replacement token (write-only).
    pub token: Option<String>,
}

impl fmt::Debug for UpdateProviderAccountTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdateProviderAccountTool")
            .field("account", &self.account)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .finish_non_exhaustive()
    }
}

impl UpdateProviderAccountTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        super::validate_account_ref(&self.account)?;
        // Only the fields given are sent: an absent field stays unchanged.
        let mut body = Map::new();
        let fields = [
            ("display_name", self.display_name.clone().map(Value::from)),
            ("enabled", self.enabled.map(Value::from)),
            ("priority", self.priority.map(Value::from)),
            ("tags", self.tags.clone().map(Value::from)),
            ("max_concurrency", self.max_concurrency.map(Value::from)),
            ("alert_threshold", self.alert_threshold.map(Value::from)),
            ("plan", self.plan.clone().map(Value::from)),
            ("token", self.token.clone().map(Value::from)),
        ];
        for (name, value) in fields
            .into_iter()
            .filter_map(|(name, value)| value.map(|v| (name, v)))
        {
            body.insert(name.to_string(), value);
        }
        let path = format!("/provider-accounts/{}", self.account);
        let account: Value = client
            .patch(&path, &Value::Object(body))
            .await
            .map_err(CallToolError::new)?;
        let text = to_string_pretty(&account).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
