//! Branch conditions for [`WorkflowContext`].
//!
//! A handler branches with plain Rust `if`/`else`, which the planner cannot
//! see. These two methods make a branch visible to
//! [`Engine::plan_handler`](crate::engine::Engine::plan_handler) without
//! changing what the handler does at run time.

use serde::de::DeserializeOwned;

use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::plan::{ConditionResult, lock_plan};

/// Why a condition computed from a step output cannot be resolved by the
/// planner.
const DYNAMIC_REASON: &str =
    "depends on a previous step's output, which is synthetic while planning";

impl WorkflowContext {
    /// Evaluate a named branch condition against the typed run input.
    ///
    /// The run payload is deserialized into `T`, exactly like
    /// [`input`](Self::input), and `predicate` decides the branch on it. `label`
    /// is a human-readable name for the branch, shown in the plan; it is never
    /// parsed nor evaluated. In plan mode the result is also recorded as
    /// [`ConditionResult::Evaluated`] on the next planned step, so the operator
    /// sees which branch the plan followed and why.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] when the run payload cannot be read, and
    /// [`EngineError::Serialization`] when it does not match `T`: a misspelled
    /// field or variant fails the branch instead of silently taking the other
    /// one.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, PartialEq)]
    /// #[serde(rename_all = "lowercase")]
    /// enum Env {
    ///     Prod,
    ///     Staging,
    /// }
    ///
    /// #[derive(Deserialize)]
    /// struct DeployInput {
    ///     env: Env,
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// if ctx.when("production run", |i: &DeployInput| i.env == Env::Prod).await? {
    ///     ctx.shell("deploy-prod", ShellConfig::new("./deploy prod")).await?;
    /// } else {
    ///     ctx.skip("deploy-prod", "not a production run").await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn when<T, F>(&mut self, label: &str, predicate: F) -> Result<bool, EngineError>
    where
        T: DeserializeOwned,
        F: FnOnce(&T) -> bool,
    {
        let input: T = self.input().await?;
        let value = predicate(&input);

        if let Some(plan) = self.plan().cloned() {
            lock_plan(&plan).set_condition(ConditionResult::Evaluated {
                expression: label.to_string(),
                value,
            });
        }

        Ok(value)
    }

    /// Record a branch condition whose value depends on a previous step's
    /// output.
    ///
    /// Returns `value` unchanged; `label` names the branch in the plan. Under
    /// planning, step outputs are synthetic,
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
    pub fn when_dynamic(&mut self, label: &str, value: bool) -> bool {
        if let Some(plan) = self.plan().cloned() {
            lock_plan(&plan).set_condition(ConditionResult::Unevaluable {
                expression: label.to_string(),
                reason: DYNAMIC_REASON.to_string(),
            });
        }

        value
    }
}
