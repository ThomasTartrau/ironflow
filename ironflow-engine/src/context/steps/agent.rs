//! Agent step for [`WorkflowContext`].

use ironflow_store::models::StepKind;

use crate::config::{AgentStep, StepConfig};
use crate::context::WorkflowContext;
use crate::error::EngineError;

impl WorkflowContext {
    /// Execute an agent step.
    ///
    /// With [`output::<T>()`](crate::config::AgentStepConfig::output) on the
    /// config, the step returns the `T` the agent answered; otherwise it
    /// returns the raw [`StepOutput`](crate::executor::StepOutput). See
    /// [`AgentStep`].
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
        let output = self
            .execute_step(
                name,
                StepKind::Agent,
                StepConfig::Agent(config.into_config()),
            )
            .await?;
        C::answer(output)
    }
}
