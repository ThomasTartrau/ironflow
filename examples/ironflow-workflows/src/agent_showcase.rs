use ironflow_engine::config::AgentStepConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};

/// Showcase workflow that exercises the full verbose agent timeline:
/// thinking blocks, multiple tool calls, tool results, and per-turn tokens.
///
/// Runs an agent step in verbose mode with tool access so the dashboard
/// timeline has something rich to render. Use this workflow to manually
/// validate the `<AgentDebugTimeline>` component.
pub struct AgentShowcase;

impl WorkflowHandler for AgentShowcase {
    fn name(&self) -> &str {
        "agent-showcase"
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Demo workflow that forces the agent to use Bash and \
                          Read tools and think about its plan, producing a rich \
                          verbose trace for the dashboard timeline."
                .to_string(),
            source_code: Some(include_str!("agent_showcase.rs").to_string()),
            sub_workflows: Vec::new(),
            category: None,
            version: self.version().to_string(),
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent(
                "explore-and-report",
                AgentStepConfig::new(
                    "Inspect the current working directory. Use `Bash` to list \
                     the top-level files (for example: `ls -la`) and \
                     `Bash` again to read any file you find interesting. \
                     Then produce a short markdown report summarising what \
                     you found. Think step by step before acting.",
                )
                .model("claude-opus-4-7")
                .allow_tool("Bash")
                .allow_tool("Read")
                .max_budget_usd(0.50)
                .max_turns(6)
                .verbose(true),
            )
            .await?;

            Ok(())
        })
    }
}
