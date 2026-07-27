use ironflow_engine::config::{AgentStepConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};
use std::collections::HashMap;

pub struct GitInsight;

impl WorkflowHandler for GitInsight {
    fn name(&self) -> &str {
        "git-insight"
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Collects git log and contributor data, then asks an AI \
                          agent to analyze repository activity and team dynamics."
                .to_string(),
            source_code: Some(include_str!("git_insight.rs").to_string()),
            sub_workflows: Vec::new(),
            category: None,
            version: self.version().map(str::to_string),
            input_schema: None,
            default_labels: HashMap::new(),
            schedule: None,
            default_max_cost_usd: None,
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Step 1-2: Collect git data
            let log = ctx
                .shell(
                    "recent-commits",
                    ShellConfig::new("git log --oneline -10 || echo 'Not a git repository'"),
                )
                .await?;
            let contributors = ctx
                .shell(
                    "contributors",
                    ShellConfig::new(
                        "git shortlog -sn --no-merges | head -5 || echo 'No contributors found'",
                    ),
                )
                .await?;

            // Step 3: AI analysis of git activity
            let log_out = log
                .output
                .get("stdout")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let contrib_out = contributors
                .output
                .get("stdout")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            ctx.agent(
                "analysis",
                AgentStepConfig::new(&format!(
                    "Analyze this git repository data:\n\n\
                     Recent Commits:\n{log_out}\n\n\
                     Contributors:\n{contrib_out}\n\n\
                     Provide: a summary of recent activity, the focus areas of development, \
                     and the team dynamics (who contributes most). Be concise and insightful."
                ))
                .model("haiku")
                .max_budget_usd(0.05)
                .max_turns(1),
            )
            .await?;

            Ok(())
        })
    }
}
