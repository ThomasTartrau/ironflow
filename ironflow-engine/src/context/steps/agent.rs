//! Agent step for [`WorkflowContext`].

use uuid::Uuid;

use ironflow_store::models::StepKind;

use crate::config::{AgentStep, StepConfig};
use crate::context::WorkflowContext;
use crate::error::EngineError;

/// What an agent step answered, with the metadata the provider attached to it.
///
/// Returned by [`WorkflowContext::agent_with_meta`]. It keeps the typed answer
/// of a step built with
/// [`output::<T>()`](crate::config::AgentStepConfig::output) next to the
/// environment and account the step ran on.
///
/// # Examples
///
/// ```
/// use ironflow_engine::context::AgentReply;
///
/// let reply = AgentReply {
///     answer: 42_u32,
///     environment_id: Some("ironflow-env-0a1b2c".to_string()),
///     account_id: None,
/// };
/// assert_eq!(reply.environment_id.as_deref(), Some("ironflow-env-0a1b2c"));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AgentReply<A> {
    /// The answer of the step: the typed `T` with `.output::<T>()`, the raw
    /// [`StepOutput`](crate::executor::StepOutput) otherwise.
    pub answer: A,
    /// Claim name to pass to
    /// [`AgentStepConfig::resume_environment`](crate::config::AgentStepConfig::resume_environment).
    /// `None` on providers without persistent environments.
    pub environment_id: Option<String>,
    /// Provider Account the step ran on. `None` when the worker environment
    /// was used.
    pub account_id: Option<Uuid>,
}

impl WorkflowContext {
    /// Execute an agent step.
    ///
    /// With [`output::<T>()`](crate::config::AgentStepConfig::output) on the
    /// config, the step returns the `T` the agent answered; otherwise it
    /// returns the raw [`StepOutput`](crate::executor::StepOutput). See
    /// [`AgentStep`]. To read the environment or account of the step as well,
    /// use [`agent_with_meta`](WorkflowContext::agent_with_meta).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the agent invocation fails or the store
    /// errors, and [`EngineError::Serialization`] if a typed answer does not
    /// match its type.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::AgentStepConfig;
    /// use ironflow_engine::error::EngineError;
    /// use schemars::JsonSchema;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, JsonSchema)]
    /// struct Review {
    ///     approved: bool,
    ///     comments: Vec<String>,
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let review = ctx
    ///     .agent(
    ///         "review",
    ///         AgentStepConfig::new("Review the code").max_turns(2).output::<Review>(),
    ///     )
    ///     .await?;
    /// if !review.approved {
    ///     println!("{} comments", review.comments.len());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn agent<C: AgentStep>(
        &mut self,
        name: &str,
        config: C,
    ) -> Result<C::Answer, EngineError> {
        self.agent_with_meta(name, config)
            .await
            .map(|reply| reply.answer)
    }

    /// Execute an agent step and return its answer with the step metadata.
    ///
    /// Behaves like [`agent`](WorkflowContext::agent), but the
    /// [`AgentReply`] also carries the `environment_id` and `account_id` of
    /// the step, which a typed answer would otherwise hide. On replay the
    /// reply carries the ids persisted with the step.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the agent invocation fails or the store
    /// errors, and [`EngineError::Serialization`] if a typed answer does not
    /// match its type.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::{AgentStepConfig, Tool};
    /// use ironflow_engine::error::EngineError;
    /// use schemars::JsonSchema;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, JsonSchema)]
    /// struct Cloned {
    ///     head: String,
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let reply = ctx
    ///     .agent_with_meta(
    ///         "clone",
    ///         AgentStepConfig::new("Clone the repository into /workspace")
    ///             .allow_tool(Tool::Bash)
    ///             .max_budget_usd(0.50)
    ///             .output::<Cloned>(),
    ///     )
    ///     .await?;
    /// if let Some(environment) = reply.environment_id.as_deref() {
    ///     ctx.agent(
    ///         "fix",
    ///         AgentStepConfig::new("Fix the failing test in /workspace")
    ///             .allow_tool(Tool::Bash)
    ///             .max_budget_usd(0.50)
    ///             .resume_environment(environment),
    ///     )
    ///     .await?;
    /// }
    /// println!("{}", reply.answer.head);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn agent_with_meta<C: AgentStep>(
        &mut self,
        name: &str,
        config: C,
    ) -> Result<AgentReply<C::Answer>, EngineError> {
        let output = self
            .execute_step(
                name,
                StepKind::Agent,
                StepConfig::Agent(config.into_config()),
            )
            .await?;
        let environment_id = output.environment_id.clone();
        let account_id = output.account_id;
        Ok(AgentReply {
            answer: C::answer(output)?,
            environment_id,
            account_id,
        })
    }
}
