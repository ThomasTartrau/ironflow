//! [`TeamWorkflow`] -- the child run of a task a supervisor delegated.

use std::marker::PhantomData;

use serde_json::Value;

use crate::config::Team;
use crate::context::WorkflowContext;

use super::{HandlerFuture, TypedWorkflow, WorkflowHandler, input_schema_for};

/// The workflow running the tasks a supervisor delegates to a [`Team`].
///
/// Its input is one variant of the team. It waits for the approval of the
/// task, if [`Team::assign`] set one, runs the role in an agent step named
/// after the role, checks the answer against the output type of the role,
/// and sets it as the run output, which the supervisor reads back. Register it next to the supervisor's workflow, on
/// the API server and on the worker, and list it in the supervisor's
/// [`sub_workflows`](WorkflowHandler::sub_workflows).
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{AgentRole, Assignment, Team};
/// use ironflow_engine::handler::{TeamWorkflow, WorkflowHandler, sub_workflow_names};
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
///
/// struct Coder;
///
/// impl AgentRole for Coder {
///     type Input = String;
///     type Output = String;
///     fn prompt(&self, input: String) -> String {
///         input
///     }
/// }
///
/// #[derive(Serialize, Deserialize, JsonSchema)]
/// enum Squad {
///     Coder(String),
/// }
///
/// impl Team for Squad {
///     const NAME: &'static str = "squad";
///     fn assign(self) -> Assignment {
///         match self {
///             Squad::Coder(input) => Coder.task(input).into(),
///         }
///     }
/// }
///
/// let members = TeamWorkflow::<Squad>::new();
/// assert_eq!(members.name(), "squad");
/// assert_eq!(sub_workflow_names(&[&members]), vec!["squad"]);
/// ```
pub struct TeamWorkflow<T> {
    team: PhantomData<fn() -> T>,
}

impl<T> TeamWorkflow<T> {
    /// The workflow of the team `T`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{Assignment, Team};
    /// # use ironflow_engine::handler::{TeamWorkflow, WorkflowHandler};
    /// # use schemars::JsonSchema;
    /// # use serde::{Deserialize, Serialize};
    /// # #[derive(Serialize, Deserialize, JsonSchema)]
    /// # enum Squad { Coder(String) }
    /// # impl Team for Squad {
    /// #     const NAME: &'static str = "squad";
    /// #     fn assign(self) -> Assignment { unimplemented!() }
    /// # }
    /// assert_eq!(TeamWorkflow::<Squad>::new().name(), "squad");
    /// ```
    pub fn new() -> Self {
        Self { team: PhantomData }
    }
}

impl<T> Default for TeamWorkflow<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Team> WorkflowHandler for TeamWorkflow<T> {
    fn name(&self) -> &str {
        T::NAME
    }

    fn description(&self) -> &str {
        "Runs a task an agent supervisor delegated to a role of its team"
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<T>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let task: T = ctx.input().await?;
            let assignment = task.assign();
            let check_answer = assignment.answer_check();
            let (config, role, approval) = assignment.into_parts();
            if let Some(approval) = approval {
                ctx.approval("approval", approval).await?;
            }
            let answer = ctx.agent(&role, config).await?;
            // The output type of the role was erased by `Assignment`: an
            // answer that is not one fails here, as `ctx.agent` would.
            check_answer(&answer.output)?;
            ctx.set_output(&answer.output)
        })
    }
}

impl<T: Team> TypedWorkflow for TeamWorkflow<T> {
    type Input = T;
}
