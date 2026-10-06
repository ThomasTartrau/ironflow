//! `create_schedule` MCP tool.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde_json::{Value, json};

use crate::client::ApiClient;

/// Create a new workflow schedule.
#[mcp_tool(
    name = "create_schedule",
    description = "Create a new periodic schedule that triggers a workflow on a cron expression, evaluated in an optional IANA timezone (default UTC). Optional catch-up (latest, all, skip) and overlap (allow, skip) policies decide what happens to occurrences missed during downtime and to occurrences that come while a run is still active. Returns the created schedule with its ID and next trigger time."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CreateScheduleTool {
    /// Name of the workflow to trigger.
    pub workflow_name: String,
    /// Cron expression (5-field standard or 6-field with seconds).
    pub cron_expression: String,
    /// JSON inputs to pass to the workflow on each trigger (as a JSON string).
    #[serde(default)]
    pub inputs: Option<String>,
    /// What to do with the occurrences missed while no server fired the
    /// schedule: `latest` (default) runs the most recent one, `all` runs each
    /// of them up to `catchup_max`, `skip` runs none.
    #[serde(default)]
    pub catchup: Option<String>,
    /// Most runs created to catch up under `catchup = all`, from 1 to 1000.
    /// Defaults to 10.
    #[serde(default)]
    pub catchup_max: Option<u32>,
    /// How far back, in seconds, a missed occurrence is still caught up, from
    /// 60 to 2592000 (30 days). Defaults to 86400 (one day).
    #[serde(default)]
    pub catchup_window_secs: Option<u32>,
    /// What to do when an occurrence comes while a run of the schedule is
    /// still active: `allow` (default) starts another run, `skip` drops the
    /// occurrence.
    #[serde(default)]
    pub overlap: Option<String>,
    /// IANA timezone the cron expression is evaluated in, e.g.
    /// `Europe/Paris`. Defaults to `UTC`.
    #[serde(default)]
    pub timezone: Option<String>,
}

impl CreateScheduleTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        let body = self.body()?;
        let schedule: Value = client
            .post("/schedules", &body)
            .await
            .map_err(CallToolError::new)?;

        let text = serde_json::to_string_pretty(&schedule).map_err(CallToolError::new)?;
        Ok(CallToolResult::text_content(vec![text.into()]))
    }

    /// The request body. Optional fields are sent only when set: the server
    /// applies the defaults and validates the values.
    fn body(&self) -> Result<Value, CallToolError> {
        let inputs: Value = match &self.inputs {
            Some(s) => serde_json::from_str(s).map_err(CallToolError::new)?,
            None => json!({}),
        };
        let mut body = json!({
            "workflow_name": self.workflow_name,
            "cron_expression": self.cron_expression,
            "inputs": inputs,
        });
        if let Some(catchup) = &self.catchup {
            body["catchup"] = json!(catchup);
        }
        if let Some(catchup_max) = self.catchup_max {
            body["catchup_max"] = json!(catchup_max);
        }
        if let Some(catchup_window_secs) = self.catchup_window_secs {
            body["catchup_window_secs"] = json!(catchup_window_secs);
        }
        if let Some(overlap) = &self.overlap {
            body["overlap"] = json!(overlap);
        }
        if let Some(timezone) = &self.timezone {
            body["timezone"] = json!(timezone);
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> CreateScheduleTool {
        CreateScheduleTool {
            workflow_name: "deploy".to_string(),
            cron_expression: "0 9 * * *".to_string(),
            inputs: None,
            catchup: None,
            catchup_max: None,
            catchup_window_secs: None,
            overlap: None,
            timezone: None,
        }
    }

    #[test]
    fn body_leaves_unset_policies_to_the_server() {
        let body = tool().body().unwrap();
        assert_eq!(
            body,
            json!({
                "workflow_name": "deploy",
                "cron_expression": "0 9 * * *",
                "inputs": {},
            })
        );
    }

    #[test]
    fn body_sends_catchup_overlap_and_timezone() {
        let body = CreateScheduleTool {
            catchup: Some("all".to_string()),
            catchup_max: Some(24),
            catchup_window_secs: Some(3600),
            overlap: Some("skip".to_string()),
            timezone: Some("Europe/Paris".to_string()),
            ..tool()
        }
        .body()
        .unwrap();
        assert_eq!(body["catchup"], "all");
        assert_eq!(body["catchup_max"], 24);
        assert_eq!(body["catchup_window_secs"], 3600);
        assert_eq!(body["overlap"], "skip");
        assert_eq!(body["timezone"], "Europe/Paris");
    }

    #[test]
    fn body_rejects_invalid_inputs_json() {
        let invalid = CreateScheduleTool {
            inputs: Some("{not json".to_string()),
            ..tool()
        };
        assert!(invalid.body().is_err());
    }
}
