//! Agent step for [`WorkflowContext`].

use ironflow_store::models::StepKind;

use crate::config::{AgentStepConfig, StepConfig};
use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::StepOutput;

impl WorkflowContext {
    /// Execute an agent step.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the agent invocation fails or the store errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::AgentStepConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let review = ctx.agent("review", AgentStepConfig::new("Review the code")).await?;
    /// println!("review: {}", review.output);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn agent(
        &mut self,
        name: &str,
        config: impl Into<AgentStepConfig>,
    ) -> Result<StepOutput, EngineError> {
        self.execute_step(name, StepKind::Agent, StepConfig::Agent(config.into()))
            .await
    }
}
