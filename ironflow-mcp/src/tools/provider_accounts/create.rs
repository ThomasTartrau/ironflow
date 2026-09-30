//! `create_provider_account` MCP tool.

use std::fmt;

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json, to_string_pretty};

use crate::client::ApiClient;

/// Add a Provider Account.
#[mcp_tool(
    name = "create_provider_account",
    description = "Add a Provider Account (kind 'claude_subscription': a Claude Pro/Max subscription token from `claude setup-token`, sk-ant-oat01-...). The token is checked against the provider first and rejected with 422 when invalid. The response never includes the token. Requires admin permissions."
)]
#[derive(serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CreateProviderAccountTool {
    /// Unique name, a lowercase slug (e.g. `perso-max`).
    pub name: String,
    /// Account kind, `claude_subscription` by default.
    pub kind: Option<String>,
    /// Account token (write-only).
    pub token: String,
    /// Human-readable name.
    pub display_name: Option<String>,
    /// Tags.
    pub tags: Option<Vec<String>>,
    /// Priority, lower is preferred.
    pub priority: Option<i32>,
    /// Maximum concurrent steps.
    pub max_concurrency: Option<u32>,
    /// Subscription plan (`pro`, `max`).
    pub plan: Option<String>,
}

impl fmt::Debug for CreateProviderAccountTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateProviderAccountTool")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl CreateProviderAccountTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let body = json!({
            "name": self.name,
            "kind": self.kind.as_deref().unwrap_or("claude_subscription"),
            "token": self.token,
            "display_name": self.display_name,
            "tags": self.tags,
            "priority": self.priority,
            "max_concurrency": self.max_concurrency,
            "plan": self.plan,
        });
        let account: Value = client
            .post("/provider-accounts", &body)
            .await
            .map_err(CallToolError::new)?;
        let text = to_string_pretty(&account).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
