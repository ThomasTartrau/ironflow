# Supervisor delegating to its team

A role that hands tasks to other roles is a supervisor: never write the loop (agent turn,
wave of children, prompt rebuilt with the results) by hand. Its output is a
`Turn<Answer, Team>`; its team is an enum, one variant per role it may delegate to, mapped
to the role by the exhaustive `match` of `Team::assign`. A role the enum does not name
cannot be a target.

```rust,no_run
use ironflow_engine::config::{
    AgentRole, ApprovalConfig, Assignment, Prompt, SupervisionLimits, Team, Turn,
};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{
    HandlerFuture, TeamWorkflow, TypedWorkflow, WorkflowHandler, sub_workflow_names,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

struct Coder;

impl AgentRole for Coder {
    type Input = String;
    type Output = String;

    fn prompt(&self, task: String) -> String {
        // The task is written by the supervisor agent: untrusted.
        Prompt::new().text("Implement:").untrusted(&task).to_string()
    }
}

struct Deployer;

impl AgentRole for Deployer {
    type Input = String;
    type Output = String;

    fn prompt(&self, task: String) -> String {
        Prompt::new().text("Deploy:").untrusted(&task).to_string()
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
enum Squad {
    Coder(String),
    Deployer(String),
}

impl Team for Squad {
    const NAME: &'static str = "squad";

    fn assign(self) -> Assignment {
        match self {
            Squad::Coder(task) => Coder.task(task).into(),
            Squad::Deployer(task) => Assignment::from(Deployer.task(task))
                .after_approval(ApprovalConfig::new("Let the agent deploy?")),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct Report {
    summary: String,
}

struct Lead;

impl AgentRole for Lead {
    type Input = String;
    type Output = Turn<Report, Squad>;

    fn prompt(&self, feature: String) -> String {
        Prompt::new()
            .text("Split this feature between your team, then report:")
            .untrusted(&feature)
            .to_string()
    }

    fn max_budget_usd(&self) -> Option<f64> {
        Some(0.50)
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct FeatureInput {
    feature: String,
}

struct Ship;

impl WorkflowHandler for Ship {
    fn name(&self) -> &str {
        "ship"
    }

    fn sub_workflows(&self) -> Vec<String> {
        sub_workflow_names(&[&TeamWorkflow::<Squad>::new()])
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: FeatureInput = ctx.input().await?;
            let report = ctx
                .supervise(
                    "lead",
                    Lead.task(input.feature),
                    SupervisionLimits::default().max_rounds(3),
                )
                .await?;
            ctx.set_output(&report.summary)
        })
    }
}

impl TypedWorkflow for Ship {
    type Input = FeatureInput;
}

/// Register the team next to its supervisor, on the server and on the worker.
fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(Ship), Box::new(TeamWorkflow::<Squad>::new())]
}
```

- Each round is an agent step `lead/turn-{n}`; each task a child run of `squad` in a step
  `lead/turn-{n}/{i}-{role}`. The children of a round run concurrently and the supervisor
  waits for all of them. A child that fails fails the run.
- A child of a task with `after_approval` waits for that approval before the agent starts;
  the supervisor suspends with it and resumes once it is approved.
- A task matching no variant is refused: a skipped step `lead/turn-{n}/{i}-refused` with a
  `DELEGATION_REFUSED` reason, no child run, and the refusal goes back to the supervisor.
- The next round gets every earlier task, result and refusal in untrusted blocks. A team
  role receives an input written by an agent: wrap it with `Prompt::untrusted`.
- `SupervisionLimits` caps the rounds and the tasks per round (5 and 5 by default); the
  children share the cost cap of the run tree and the workflow guard. A turn with both
  tasks and an answer, or neither, fails with `EngineError::SupervisionFailed`.
- `TeamWorkflow::<Squad>` not registered: the first delegation fails with
  `no handler registered: squad`.
- Finished rounds and children are replayed on resume: no turn or child runs twice.
