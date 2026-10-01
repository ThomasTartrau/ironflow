//! `list_signals` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::to_string_pretty;

use crate::client::ApiClient;

/// List received signals.
#[mcp_tool(
    name = "list_signals",
    description = "List the signals Ironflow received, newest first, with their name, key, payload and reception time. Filter by exact name and key. Paginated: the response meta carries page, per_page and total."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListSignalsTool {
    /// Only signals with this exact name.
    pub name: Option<String>,
    /// Only signals with this exact key.
    pub key: Option<String>,
    /// Page number (1-based, default: 1).
    pub page: Option<u32>,
    /// Items per page (default: 20, max: 100).
    pub per_page: Option<u32>,
}

impl ListSignalsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref name) = self.name {
            query.push(("name", name.clone()));
        }
        if let Some(ref key) = self.key {
            query.push(("key", key.clone()));
        }
        if let Some(page) = self.page {
            query.push(("page", page.to_string()));
        }
        if let Some(per_page) = self.per_page {
            query.push(("per_page", per_page.to_string()));
        }

        let params: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let result = client
            .get_raw_with_query("/signals", &params)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
