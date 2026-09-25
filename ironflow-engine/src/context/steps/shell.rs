//! Shell step for [`WorkflowContext`].

use ironflow_store::models::StepKind;

use crate::config::{ShellConfig, StepConfig};
use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::StepOutput;

impl WorkflowContext {
    /// Execute a shell step.
    ///
    /// Creates the step record, runs the command, persists the result,
    /// and returns the output for use in subsequent steps.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the command fails or the store errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let files = ctx.shell("list", ShellConfig::new("ls -la")).await?;
    /// println!("stdout: {}", files.stdout());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shell(
        &mut self,
        name: &str,
        config: ShellConfig,
    ) -> Result<StepOutput, EngineError> {
        self.execute_step(name, StepKind::Shell, StepConfig::Shell(config))
            .await
    }
}
