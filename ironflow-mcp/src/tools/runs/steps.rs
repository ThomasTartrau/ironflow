//! `list_agent_role_steps` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, to_string_pretty};

use crate::client::ApiClient;

/// List the agent steps of one agent role across every run and workflow.
#[mcp_tool(
    name = "list_agent_role_steps",
    description = "List the agent steps run by one agent role (AgentRole name, exact match) across every workflow and run, newest first. Returns a paginated list of steps with their run, status, cost and duration."
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListAgentRoleStepsTool {
    /// Name of the agent role (`AgentRole::name`), matched exactly.
    pub agent_role: String,
    /// Page number (1-based, default: 1).
    pub page: Option<u32>,
    /// Items per page (default: 20, max: 100).
    pub per_page: Option<u32>,
}

impl ListAgentRoleStepsTool {
    /// Execute the tool against the Ironflow API.
    ///
    /// # Errors
    ///
    /// Returns [`CallToolError`] on API failure or serialization error.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut query: Vec<(&str, String)> = vec![("agent_role", self.agent_role.clone())];
        if let Some(p) = self.page {
            query.push(("page", p.to_string()));
        }
        if let Some(pp) = self.per_page {
            query.push(("per_page", pp.to_string()));
        }

        let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let result: Value = client
            .get_raw_with_query("/steps", &query_refs)
            .await
            .map_err(CallToolError::new)?;

        let text = to_string_pretty(&result).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
