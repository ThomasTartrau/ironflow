use ironflow_engine::config::AgentStepConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};
use serde_json::json;
use std::collections::HashMap;

use super::Collect;

/// Workflow B: runs the collect sub-workflow, then enriches the raw data
/// with an AI-generated structured summary.
pub struct Enrich;

impl WorkflowHandler for Enrich {
    fn name(&self) -> &str {
        "pipeline-enrich"
    }

    fn category(&self) -> Option<&str> {
        Some("examples/pipeline")
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Calls pipeline-collect to gather raw metrics, then uses an AI agent \
                          to parse them into a structured JSON summary."
                .to_string(),
            source_code: Some(include_str!("enrich.rs").to_string()),
            sub_workflows: vec!["pipeline-collect".to_string()],
            category: Some("examples/pipeline".to_string()),
            version: self.version().map(str::to_string),
            input_schema: None,
            default_labels: HashMap::new(),
            schedule: None,
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let collect_result = ctx.workflow(&Collect, json!({})).await?;

            let child_run_id = collect_result
                .output
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let steps = ctx
                .store()
                .list_steps(child_run_id.parse().unwrap_or_default())
                .await?;

            let raw_data: String = steps
                .iter()
                .filter_map(|s| {
                    let name = &s.name;
                    let stdout = s
                        .output
                        .as_ref()
                        .and_then(|o| o.get("stdout"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if stdout.is_empty() {
                        None
                    } else {
                        Some(format!("=== {name} ===\n{stdout}"))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            ctx.agent(
                "structure",
                AgentStepConfig::new(&format!(
                    "Here is raw system output:\n\n{raw_data}\n\n\
                     Parse this into a JSON object with keys: \
                     disk_usage_percent, memory_free_pages, uptime_days. \
                     Return ONLY the JSON, no markdown."
                ))
                .model("haiku")
                .max_budget_usd(0.10)
                .max_turns(1),
            )
            .await?;

            Ok(())
        })
    }
}
