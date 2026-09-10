//! `search_runs` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::to_string_pretty;

use crate::client::ApiClient;

/// Search workflow runs with advanced filters.
#[mcp_tool(
    name = "search_runs",
    description = "Search workflow runs with advanced filters: workflow name, status, labels, step presence, author, and pagination. Superset of list_runs with additional filtering capabilities."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SearchRunsTool {
    /// Filter by workflow name.
    pub workflow: Option<String>,
    /// Filter by run status (e.g. `completed`, `failed`, `running`, `pending`).
    pub status: Option<String>,
    /// Filter by labels. Comma-separated `key:value` pairs (e.g. `env:prod,team:backend`).
    pub label: Option<String>,
    /// Filter by step presence (only applies to completed/cancelled runs).
    /// When true, only return runs that have steps. When false, only runs without steps.
    pub has_steps: Option<bool>,
    /// Filter by author: the user ID (UUID) that triggered the run.
    pub created_by: Option<String>,
    /// Page number (1-based).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
}

impl SearchRunsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref workflow) = self.workflow {
            query.push(("workflow", workflow.clone()));
        }
        if let Some(ref status) = self.status {
            query.push(("status", status.clone()));
        }
        if let Some(ref label) = self.label {
            query.push(("label", label.clone()));
        }
        if let Some(has_steps) = self.has_steps {
            query.push(("has_steps", has_steps.to_string()));
        }
        if let Some(ref created_by) = self.created_by {
            query.push(("created_by", created_by.clone()));
        }
        if let Some(page) = self.page {
            query.push(("page", page.to_string()));
        }
        if let Some(per_page) = self.per_page {
            query.push(("per_page", per_page.to_string()));
        }

        let params: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let result = client
            .get_raw_with_query("/runs", &params)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
