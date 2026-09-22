//! Branch conditions for [`WorkflowContext`].
//!
//! A handler branches with plain Rust `if`/`else`, which the planner cannot
//! see. These two methods make a branch visible to
//! [`Engine::plan_handler`](crate::engine::Engine::plan_handler) without
//! changing what the handler does at run time.

use serde_json::Value;

use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::plan::{ConditionResult, lock_plan};

/// Why a condition computed from a step output cannot be resolved by the
/// planner.
const DYNAMIC_REASON: &str =
    "depends on a previous step's output, which is synthetic while planning";

impl WorkflowContext {
    /// Evaluate a named branch condition against the run input.
    ///
    /// Outside plan mode this simply applies `predicate` to
    /// [`payload`](Self::payload). In plan mode the result is also recorded as
    /// [`ConditionResult::Evaluated`] on the next planned step, so the operator
    /// sees which branch the plan followed and why.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] when the run payload cannot be read.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// if ctx.when("input.env == 'prod'", |p| p["env"] == "prod").await? {
    ///     ctx.shell("deploy-prod", ShellConfig::new("./deploy prod")).await?;
    /// } else {
    ///     ctx.skip("deploy-prod", "not a production run").await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn when<F>(&mut self, expression: &str, predicate: F) -> Result<bool, EngineError>
    where
        F: FnOnce(&Value) -> bool,
    {
        let payload = self.payload().await?;
        let value = predicate(&payload);

        if let Some(plan) = self.plan().cloned() {
            lock_plan(&plan).set_condition(ConditionResult::Evaluated {
                expression: expression.to_string(),
                value,
            });
        }

        Ok(value)
    }

    /// Record a branch condition whose value depends on a previous step's
    /// output.
    ///
    /// Returns `value` unchanged. Under planning, step outputs are synthetic,
    /// so the condition is recorded as [`ConditionResult::Unevaluable`]: the
    /// plan still follows the branch the synthetic output produces, and the
    /// operator is told the other branch may run instead.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let build = ctx.shell("build", ShellConfig::new("cargo build")).await?;
    /// if ctx.when_dynamic("build succeeded", build.is_success()) {
    ///     ctx.shell("deploy", ShellConfig::new("./deploy")).await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn when_dynamic(&mut self, expression: &str, value: bool) -> bool {
        if let Some(plan) = self.plan().cloned() {
            lock_plan(&plan).set_condition(ConditionResult::Unevaluable {
                expression: expression.to_string(),
                reason: DYNAMIC_REASON.to_string(),
            });
        }

        value
    }
}
