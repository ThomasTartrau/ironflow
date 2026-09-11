//! `get_stats` and `get_stats_history` MCP tools.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::Value;

use crate::client::ApiClient;

/// Get aggregated statistics about workflow executions.
#[mcp_tool(
    name = "get_stats",
    description = "Get aggregated statistics about all workflow executions: total runs, completed, failed, active, success rate, total cost, and total duration."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetStatsTool {}

impl GetStatsTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let stats: Value = client.get("/stats").await.map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&stats).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}

/// Get time-bucketed historical statistics for trend charts.
#[mcp_tool(
    name = "get_stats_history",
    description = "Get time-bucketed historical statistics: run counts by status, average and p95 duration, cost per time bucket. Supports workflow filter, period (24h/7d/30d/90d), and granularity (1h/1d/1w)."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetStatsHistoryTool {
    /// Filter by workflow name. Omit to aggregate all workflows.
    #[serde(default)]
    pub workflow: Option<String>,
    /// Time period: 24h, 7d, 30d, 90d. Defaults to 7d.
    #[serde(default)]
    pub period: Option<String>,
    /// Bucket granularity: 1h, 1d, 1w. Auto-derived from period when omitted.
    #[serde(default)]
    pub granularity: Option<String>,
}

impl GetStatsHistoryTool {
    /// Execute the tool against the Ironflow API.
    ///
    /// # Errors
    ///
    /// Returns [`CallToolError`] on API failure or serialization error.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let mut params: Vec<(&str, &str)> = Vec::new();
        if let Some(ref w) = self.workflow {
            params.push(("workflow", w.as_str()));
        }
        if let Some(ref p) = self.period {
            params.push(("period", p.as_str()));
        }
        if let Some(ref g) = self.granularity {
            params.push(("granularity", g.as_str()));
        }

        let history: Value = client
            .get_raw_with_query("/stats/history", &params)
            .await
            .map_err(CallToolError::new)?;
        let text = serde_json::to_string_pretty(&history).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}
