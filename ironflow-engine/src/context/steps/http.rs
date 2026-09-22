//! HTTP step for [`WorkflowContext`].

use ironflow_store::models::StepKind;

use crate::config::{HttpConfig, StepConfig};
use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::StepOutput;

impl WorkflowContext {
    /// Execute an HTTP step.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the request fails or the store errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::HttpConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let resp = ctx.http("health", HttpConfig::get("https://api.example.com/health")).await?;
    /// println!("status: {}", resp.output["status"]);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn http(
        &mut self,
        name: &str,
        config: HttpConfig,
    ) -> Result<StepOutput, EngineError> {
        self.execute_step(name, StepKind::Http, StepConfig::Http(config))
            .await
    }
}
