//! Reusable agent with a fixed role.
//!
//! An [`AgentRole`] is an agent defined once: its persona, tools, rights and
//! output type. A workflow calls it with
//! `ctx.agent("step", Role.task(input))`. The task is an [`AgentStep`], so it is
//! persisted, replayed and run by the worker exactly like a hand-built
//! [`AgentStepConfig`].

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use ironflow_core::agent_role::{AgentRoleBinding, RoleCostCap};
use ironflow_core::operations::agent::{Effort, PermissionMode};
use ironflow_core::retry::RetryPolicy;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use super::agent::{AgentStep, AgentStepConfig, Tool};
use crate::error::EngineError;
use crate::executor::StepOutput;

/// A reusable agent with a fixed role: persona, tools, rights and output type.
///
/// Only [`prompt`](Self::prompt) is required. The persona, tools, permission
/// mode, MCP config and output type are locked by the role: the caller of
/// [`task`](Self::task) cannot change them (a role with more rights is another
/// role). The model, budget, turn limit, effort and per-call settings are
/// defaults the caller may override on the [`AgentTask`].
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{AgentRole, AgentStep, Tool};
/// use schemars::JsonSchema;
/// use serde::Deserialize;
///
/// #[derive(Deserialize, JsonSchema)]
/// struct Verdict {
///     approved: bool,
/// }
///
/// struct Reviewer;
///
/// impl AgentRole for Reviewer {
///     type Input = String;
///     type Output = Verdict;
///
///     fn prompt(&self, input: String) -> String {
///         format!("Review: {input}")
///     }
///
///     fn append_system_prompt(&self) -> Option<&str> {
///         Some("You are a strict reviewer.")
///     }
///
///     fn tools(&self) -> Vec<Tool> {
///         vec![Tool::Read]
///     }
///
///     fn max_turns(&self) -> Option<u32> {
///         Some(4)
///     }
/// }
///
/// let config = Reviewer.task("diff".to_string()).max_budget_usd(0.10).into_config();
/// assert_eq!(config.append_system_prompt.as_deref(), Some("You are a strict reviewer."));
/// assert_eq!(config.allowed_tools, vec!["Read"]);
/// assert!(config.json_schema.is_some());
/// assert_eq!(config.max_budget_usd, Some(0.10));
/// ```
///
/// Calling it from a workflow:
///
/// ```no_run
/// # use ironflow_engine::config::AgentRole;
/// # use ironflow_engine::context::WorkflowContext;
/// # use ironflow_engine::error::EngineError;
/// # use schemars::JsonSchema;
/// # use serde::Deserialize;
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Verdict { approved: bool }
/// # struct Reviewer;
/// # impl AgentRole for Reviewer {
/// #     type Input = String;
/// #     type Output = Verdict;
/// #     fn prompt(&self, input: String) -> String { input }
/// # }
/// # async fn example(ctx: &mut WorkflowContext, diff: String) -> Result<(), EngineError> {
/// let verdict: Verdict = ctx.agent("review", Reviewer.task(diff)).await?;
/// # Ok(())
/// # }
/// ```
pub trait AgentRole {
    /// What the caller passes to [`task`](Self::task) to build the prompt.
    type Input;

    /// What the role answers; `ctx.agent` returns it.
    type Output: DeserializeOwned + JsonSchema;

    /// The prompt of one task. Required.
    fn prompt(&self, input: Self::Input) -> String;

    /// Replaces the provider system prompt. Claude Code loses its own (tools,
    /// skills) when it is replaced. `None` by default.
    fn system_prompt(&self) -> Option<&str> {
        None
    }

    /// Persona appended to the provider system prompt. Prefer it to
    /// [`system_prompt`](Self::system_prompt) for a role with tools. `None` by
    /// default.
    fn append_system_prompt(&self) -> Option<&str> {
        None
    }

    /// Tools the role may use. None by default.
    fn tools(&self) -> Vec<Tool> {
        Vec::new()
    }

    /// Permission mode of the role; `None` keeps the
    /// `AgentConfig::new` default.
    fn permission_mode(&self) -> Option<PermissionMode> {
        None
    }

    /// MCP configuration, as JSON or a path. `None` by default.
    fn mcp_config(&self) -> Option<&str> {
        None
    }

    /// Restrict the MCP servers to [`mcp_config`](Self::mcp_config). `false` by
    /// default.
    fn strict_mcp_config(&self) -> bool {
        false
    }

    /// Default model; `None` keeps the `AgentConfig::new` default. The
    /// caller may override it.
    fn model(&self) -> Option<&str> {
        None
    }

    /// Default budget in USD. The caller may override it.
    fn max_budget_usd(&self) -> Option<f64> {
        None
    }

    /// Default turn limit. The caller may override it.
    ///
    /// A structured output needs at least 2 turns; like [`AgentStepConfig`],
    /// the role does not validate it.
    fn max_turns(&self) -> Option<u32> {
        None
    }

    /// Default effort. The caller may override it.
    fn effort(&self) -> Option<Effort> {
        None
    }

    /// Name identifying the role on the platform. `None` by default: the role
    /// is not tracked.
    ///
    /// Steps of roles with the same name share one cost counter; renaming a
    /// role starts a new counter. Required by [`cost_cap`](Self::cost_cap).
    /// Must be valid per [`is_valid_role_name`](ironflow_core::agent_role::is_valid_role_name):
    /// ASCII lowercase letters, digits, `-`, `_` and `.`, at most 64 bytes.
    fn name(&self) -> Option<&str> {
        None
    }

    /// Cost cap of the role over a calendar period (UTC), across every run of
    /// every workflow. `None` by default.
    ///
    /// Checked by the engine before each agent step of the role: once the cap
    /// is reached, the step fails with `AgentError::RoleCostCapReached`
    /// without calling the provider. A step already running may overshoot it
    /// by at most its own `max_budget_usd`; a step without one holds all the
    /// remaining room while it runs. An operator may lower the cap from the
    /// API, never raise it.
    fn cost_cap(&self) -> Option<RoleCostCap> {
        None
    }

    /// Build one task of this role.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { format!("Review: {input}") }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).into_config();
    /// assert_eq!(config.prompt, "Review: diff");
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the role declares a [`cost_cap`](Self::cost_cap) without a
    /// [`name`](Self::name), or a name that is not valid.
    fn task(&self, input: Self::Input) -> AgentTask<Self::Output> {
        AgentTask::from_role(self, input)
    }
}

/// One call of an [`AgentRole`], answering with the `T` of the role.
///
/// The role locks its persona, tools, permission mode, MCP config and output
/// type: none of them has a setter here, and each of these calls does not
/// compile.
///
/// The name and cost cap of the role are locked too: no setter of the task
/// changes or removes them, so every task of the role is charged to its counter.
///
/// The system prompt is locked:
///
/// ```compile_fail,E0599
/// # use ironflow_engine::config::AgentRole;
/// # use schemars::JsonSchema;
/// # use serde::Deserialize;
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Verdict { approved: bool }
/// # struct Reviewer;
/// # impl AgentRole for Reviewer {
/// #     type Input = String;
/// #     type Output = Verdict;
/// #     fn prompt(&self, input: String) -> String { input }
/// # }
/// let _ = Reviewer.task("diff".to_string()).system_prompt("x");
/// ```
///
/// The persona is locked:
///
/// ```compile_fail,E0599
/// # use ironflow_engine::config::AgentRole;
/// # use schemars::JsonSchema;
/// # use serde::Deserialize;
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Verdict { approved: bool }
/// # struct Reviewer;
/// # impl AgentRole for Reviewer {
/// #     type Input = String;
/// #     type Output = Verdict;
/// #     fn prompt(&self, input: String) -> String { input }
/// # }
/// let _ = Reviewer.task("diff".to_string()).append_system_prompt("x");
/// ```
///
/// The tools are locked:
///
/// ```compile_fail,E0599
/// # use ironflow_engine::config::{AgentRole, Tool};
/// # use schemars::JsonSchema;
/// # use serde::Deserialize;
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Verdict { approved: bool }
/// # struct Reviewer;
/// # impl AgentRole for Reviewer {
/// #     type Input = String;
/// #     type Output = Verdict;
/// #     fn prompt(&self, input: String) -> String { input }
/// # }
/// let _ = Reviewer.task("diff".to_string()).allow_tool(Tool::Bash);
/// ```
///
/// The permission mode is locked:
///
/// ```compile_fail,E0599
/// # use ironflow_core::operations::agent::PermissionMode;
/// # use ironflow_engine::config::AgentRole;
/// # use schemars::JsonSchema;
/// # use serde::Deserialize;
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Verdict { approved: bool }
/// # struct Reviewer;
/// # impl AgentRole for Reviewer {
/// #     type Input = String;
/// #     type Output = Verdict;
/// #     fn prompt(&self, input: String) -> String { input }
/// # }
/// let _ = Reviewer
///     .task("diff".to_string())
///     .permission_mode(PermissionMode::BypassPermissions);
/// ```
///
/// The MCP config is locked:
///
/// ```compile_fail,E0599
/// # use ironflow_engine::config::AgentRole;
/// # use schemars::JsonSchema;
/// # use serde::Deserialize;
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Verdict { approved: bool }
/// # struct Reviewer;
/// # impl AgentRole for Reviewer {
/// #     type Input = String;
/// #     type Output = Verdict;
/// #     fn prompt(&self, input: String) -> String { input }
/// # }
/// let _ = Reviewer.task("diff".to_string()).mcp_config("{}");
/// ```
///
/// The output type is locked:
///
/// ```compile_fail,E0599
/// # use ironflow_engine::config::AgentRole;
/// # use schemars::JsonSchema;
/// # use serde::Deserialize;
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Verdict { approved: bool }
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Other { n: u8 }
/// # struct Reviewer;
/// # impl AgentRole for Reviewer {
/// #     type Input = String;
/// #     type Output = Verdict;
/// #     fn prompt(&self, input: String) -> String { input }
/// # }
/// let _ = Reviewer.task("diff".to_string()).output::<Other>();
/// ```
///
/// Taking the raw config with [`AgentStep::into_config`] and editing it loses
/// the typed answer (`ctx.agent` then returns a [`StepOutput`]), so there is no
/// silent override.
///
/// A call-site override changes the step config, so it changes the hash of the
/// `RecordReplayProvider` fixture: a fixture recorded for the role defaults
/// does not match an overridden task.
///
/// # Examples
///
/// ```
/// # use ironflow_engine::config::{AgentRole, AgentStep};
/// # use schemars::JsonSchema;
/// # use serde::Deserialize;
/// # #[derive(Deserialize, JsonSchema)]
/// # struct Verdict { approved: bool }
/// # struct Reviewer;
/// # impl AgentRole for Reviewer {
/// #     type Input = String;
/// #     type Output = Verdict;
/// #     fn prompt(&self, input: String) -> String { input }
/// # }
/// let task = Reviewer.task("diff".to_string()).model("haiku").allow_failure();
/// let config = task.into_config();
/// assert_eq!(config.model, "haiku");
/// assert!(config.allow_failure);
/// ```
pub struct AgentTask<T> {
    config: AgentStepConfig,
    _output: PhantomData<fn() -> T>,
}

// Manual impls: a derive would require `T: Debug` / `T: Clone`, though `T` is
// only a marker (same reasoning as `WithSchema` in `ironflow-core`).
impl<T> fmt::Debug for AgentTask<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentTask")
            .field("config", &self.config)
            .finish()
    }
}

impl<T> Clone for AgentTask<T> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            _output: PhantomData,
        }
    }
}

impl<T: JsonSchema> AgentTask<T> {
    /// Builds the task from the role defaults. `pub(crate)`: no other crate can
    /// wrap an arbitrary config in an [`AgentTask`].
    ///
    /// # Panics
    ///
    /// Panics if the role declares a cost cap without a name, or an invalid name.
    pub(crate) fn from_role<R: AgentRole<Output = T> + ?Sized>(role: &R, input: R::Input) -> Self {
        let mut config = AgentStepConfig::new(&role.prompt(input));
        if let Some(prompt) = role.system_prompt() {
            config = config.system_prompt(prompt);
        }
        if let Some(prompt) = role.append_system_prompt() {
            config = config.append_system_prompt(prompt);
        }
        if let Some(mcp) = role.mcp_config() {
            config = config.mcp_config(mcp);
        }
        if role.strict_mcp_config() {
            config = config.strict_mcp_config(true);
        }
        if let Some(model) = role.model() {
            config = config.model(model);
        }
        if let Some(budget) = role.max_budget_usd() {
            config = config.max_budget_usd(budget);
        }
        if let Some(turns) = role.max_turns() {
            config = config.max_turns(turns);
        }
        if let Some(effort) = role.effort() {
            config = config.effort(effort);
        }
        if let Some(mode) = role.permission_mode() {
            config = config.permission_mode(mode);
        }
        for tool in role.tools() {
            config = config.allow_tool(tool).into();
        }
        match (role.name(), role.cost_cap()) {
            (Some(name), cap) => config = config.agent_role(AgentRoleBinding::new(name, cap)),
            (None, Some(_)) => panic!("an AgentRole with a cost_cap must declare a name"),
            (None, None) => {}
        }
        config = config.output::<T>().into();
        Self {
            config,
            _output: PhantomData,
        }
    }
}

impl<T> AgentTask<T> {
    /// Override the model of the role for this call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).model("haiku").into_config();
    /// assert_eq!(config.model, "haiku");
    /// ```
    pub fn model(mut self, model: &str) -> Self {
        self.config = self.config.model(model);
        self
    }

    /// Override the budget of the role for this call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).max_budget_usd(0.10).into_config();
    /// assert_eq!(config.max_budget_usd, Some(0.10));
    /// ```
    pub fn max_budget_usd(mut self, budget: f64) -> Self {
        self.config = self.config.max_budget_usd(budget);
        self
    }

    /// Override the turn limit of the role for this call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).max_turns(6).into_config();
    /// assert_eq!(config.max_turns, Some(6));
    /// ```
    pub fn max_turns(mut self, turns: u32) -> Self {
        self.config = self.config.max_turns(turns);
        self
    }

    /// Override the effort of the role for this call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use ironflow_core::operations::agent::Effort;
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).effort(Effort::High).into_config();
    /// assert!(matches!(config.effort, Some(Effort::High)));
    /// ```
    pub fn effort(mut self, effort: Effort) -> Self {
        self.config = self.config.effort(effort);
        self
    }

    /// Set the working directory of the call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).working_dir("/repo").into_config();
    /// assert_eq!(config.working_dir.as_deref(), Some("/repo"));
    /// ```
    pub fn working_dir(mut self, dir: &str) -> Self {
        self.config = self.config.working_dir(dir);
        self
    }

    /// Resume a Claude Code session.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).resume("sess").into_config();
    /// assert_eq!(config.resume_session_id.as_deref(), Some("sess"));
    /// ```
    pub fn resume(mut self, session_id: &str) -> Self {
        self.config = self.config.resume(session_id);
        self
    }

    /// Pin the session id of the call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).session_id("sid").into_config();
    /// assert_eq!(config.session_id.as_deref(), Some("sid"));
    /// ```
    pub fn session_id(mut self, session_id: &str) -> Self {
        self.config = self.config.session_id(session_id);
        self
    }

    /// Set the prompt sent when the step is resumed after a lost lease.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).resume_prompt("go on").into_config();
    /// assert_eq!(config.resume_prompt.as_deref(), Some("go on"));
    /// ```
    pub fn resume_prompt(mut self, prompt: &str) -> Self {
        self.config = self.config.resume_prompt(prompt);
        self
    }

    /// Resume the environment handed out by an earlier step.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).resume_environment("env-1").into_config();
    /// assert_eq!(config.resume_environment_id.as_deref(), Some("env-1"));
    /// ```
    pub fn resume_environment(mut self, environment_id: &str) -> Self {
        self.config = self.config.resume_environment(environment_id);
        self
    }

    /// Set the retry policy of the step.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use ironflow_core::retry::RetryPolicy;
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).retry_policy(RetryPolicy::new(2)).into_config();
    /// assert_eq!(config.retry.map(|r| r.max_retries()), Some(2));
    /// ```
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.config = self.config.retry_policy(policy);
        self
    }

    /// Let the run go on when this step fails.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).allow_failure().into_config();
    /// assert!(config.allow_failure);
    /// ```
    pub fn allow_failure(mut self) -> Self {
        self.config = self.config.allow_failure();
        self
    }

    /// Record the agent conversation of the call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).verbose(true).into_config();
    /// assert!(config.verbose);
    /// ```
    pub fn verbose(mut self, enabled: bool) -> Self {
        self.config = self.config.verbose(enabled);
        self
    }

    /// Add a label to the pod running the call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let config = Reviewer.task("diff".to_string()).pod_label("team", "qa").into_config();
    /// assert_eq!(config.pod_labels.get("team").map(String::as_str), Some("qa"));
    /// ```
    pub fn pod_label(mut self, key: &str, value: &str) -> Self {
        self.config = self.config.pod_label(key, value);
        self
    }

    /// Set the labels of the pod running the call.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, AgentStep};
    /// # use std::collections::BTreeMap;
    /// # use schemars::JsonSchema;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize, JsonSchema)]
    /// # struct Verdict { approved: bool }
    /// # struct Reviewer;
    /// # impl AgentRole for Reviewer {
    /// #     type Input = String;
    /// #     type Output = Verdict;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let labels = BTreeMap::from([("team".to_string(), "qa".to_string())]);
    /// let config = Reviewer.task("diff".to_string()).pod_labels(labels.clone()).into_config();
    /// assert_eq!(config.pod_labels, labels);
    /// ```
    pub fn pod_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.config = self.config.pod_labels(labels);
        self
    }
}

impl<T: DeserializeOwned> AgentStep for AgentTask<T> {
    type Answer = T;

    fn into_config(self) -> AgentStepConfig {
        self.config
    }

    fn answer(output: StepOutput) -> Result<T, EngineError> {
        output.json()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::StepArtifacts;
    use ironflow_core::agent_role::CostPeriod;
    use rust_decimal::Decimal;
    use serde::Deserialize;
    use serde_json::{Value, json, to_value};

    #[derive(Debug, PartialEq, Deserialize, JsonSchema)]
    struct Verdict {
        approved: bool,
    }

    /// Implements only the required method.
    struct Bare;

    impl AgentRole for Bare {
        type Input = String;
        type Output = Verdict;

        fn prompt(&self, input: String) -> String {
            input
        }
    }

    /// Locks MCP in strict mode.
    struct Strict;

    impl AgentRole for Strict {
        type Input = String;
        type Output = Verdict;

        fn prompt(&self, input: String) -> String {
            input
        }

        fn mcp_config(&self) -> Option<&str> {
            Some("{}")
        }

        fn strict_mcp_config(&self) -> bool {
            true
        }
    }

    /// Sets every default the other test roles leave out.
    struct Full;

    impl AgentRole for Full {
        type Input = String;
        type Output = Verdict;

        fn prompt(&self, input: String) -> String {
            input
        }

        fn system_prompt(&self) -> Option<&str> {
            Some("base prompt")
        }

        fn append_system_prompt(&self) -> Option<&str> {
            Some("persona")
        }

        fn max_budget_usd(&self) -> Option<f64> {
            Some(0.5)
        }

        fn effort(&self) -> Option<Effort> {
            Some(Effort::Low)
        }
    }

    /// Named and capped at 2 USD a day.
    struct Capped;

    impl AgentRole for Capped {
        type Input = String;
        type Output = Verdict;

        fn prompt(&self, input: String) -> String {
            input
        }

        fn name(&self) -> Option<&str> {
            Some("reviewer")
        }

        fn cost_cap(&self) -> Option<RoleCostCap> {
            Some(RoleCostCap::per_day(Decimal::TWO))
        }
    }

    /// Named, without a cap: its cost is only counted.
    struct Named;

    impl AgentRole for Named {
        type Input = String;
        type Output = Verdict;

        fn prompt(&self, input: String) -> String {
            input
        }

        fn name(&self) -> Option<&str> {
            Some("triage")
        }
    }

    /// Capped without a name: rejected.
    struct Anonymous;

    impl AgentRole for Anonymous {
        type Input = String;
        type Output = Verdict;

        fn prompt(&self, input: String) -> String {
            input
        }

        fn cost_cap(&self) -> Option<RoleCostCap> {
            Some(RoleCostCap::per_month(Decimal::ONE))
        }
    }

    /// Named with a name that is not valid.
    struct BadName;

    impl AgentRole for BadName {
        type Input = String;
        type Output = Verdict;

        fn prompt(&self, input: String) -> String {
            input
        }

        fn name(&self) -> Option<&str> {
            Some("Code Reviewer")
        }
    }

    #[test]
    fn a_named_role_binds_its_tasks_with_its_cap_and_period() {
        let config = Capped.task("p".to_string()).into_config();
        assert_eq!(
            config.agent_role,
            Some(AgentRoleBinding {
                name: "reviewer".to_string(),
                cost_cap_usd: Some(Decimal::TWO),
                period: CostPeriod::Day,
            })
        );
    }

    #[test]
    fn a_role_without_name_is_not_bound() {
        assert_eq!(Bare.task("p".to_string()).into_config().agent_role, None);
    }

    #[test]
    fn a_named_role_without_cap_counts_over_a_month() {
        let binding = Named
            .task("p".to_string())
            .into_config()
            .agent_role
            .expect("named role is bound");
        assert_eq!(binding.name, "triage");
        assert_eq!(binding.cost_cap_usd, None);
        assert_eq!(binding.period, CostPeriod::Month);
    }

    #[test]
    #[should_panic(expected = "must declare a name")]
    fn a_cost_cap_without_name_panics() {
        let _ = Anonymous.task("p".to_string());
    }

    #[test]
    #[should_panic(expected = "invalid agent role name")]
    fn an_invalid_role_name_panics() {
        let _ = BadName.task("p".to_string());
    }

    #[test]
    fn call_site_overrides_keep_the_binding() {
        let config = Capped
            .task("p".to_string())
            .max_budget_usd(5.0)
            .model("haiku")
            .allow_failure()
            .into_config();
        assert_eq!(config.max_budget_usd, Some(5.0));
        assert_eq!(
            config.agent_role.map(|b| b.name),
            Some("reviewer".to_string())
        );
    }

    fn step_output(output: Value) -> StepOutput {
        StepOutput {
            output,
            duration_ms: 0,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
            artifacts: StepArtifacts::default(),
            account_id: None,
            environment_id: None,
        }
    }

    #[test]
    fn a_role_with_only_a_prompt_takes_the_agent_config_new_defaults() {
        let task = Bare.task("p".to_string());
        let expected = AgentStepConfig::new("p").output::<Verdict>().into_config();
        assert_eq!(
            to_value(task.into_config()).expect("serialize"),
            to_value(expected).expect("serialize")
        );
    }

    #[test]
    fn a_role_without_tools_has_no_allowed_tools_and_keeps_its_schema() {
        let config = Bare.task("p".to_string()).into_config();
        assert!(config.allowed_tools.is_empty());
        assert!(
            config
                .json_schema
                .as_deref()
                .is_some_and(|schema| schema.contains("approved"))
        );
    }

    #[test]
    fn strict_mcp_config_of_the_role_is_applied() {
        let config = Strict.task("p".to_string()).into_config();
        assert!(config.strict_mcp_config);
        assert_eq!(config.mcp_config.as_deref(), Some("{}"));
        assert!(!Bare.task("p".to_string()).into_config().strict_mcp_config);
    }

    #[test]
    fn role_defaults_for_prompts_budget_and_effort_are_applied() {
        let config = Full.task("p".to_string()).into_config();
        assert_eq!(config.system_prompt.as_deref(), Some("base prompt"));
        assert_eq!(config.append_system_prompt.as_deref(), Some("persona"));
        assert_eq!(config.max_budget_usd, Some(0.5));
        assert_eq!(config.effort, Some(Effort::Low));
        let overridden = Full
            .task("p".to_string())
            .max_budget_usd(1.5)
            .effort(Effort::High)
            .into_config();
        assert_eq!(overridden.max_budget_usd, Some(1.5));
        assert_eq!(overridden.effort, Some(Effort::High));
        assert_eq!(overridden.system_prompt.as_deref(), Some("base prompt"));
    }

    #[test]
    fn task_answer_reads_the_typed_output() {
        let answer =
            <AgentTask<Verdict> as AgentStep>::answer(step_output(json!({"approved": true})))
                .expect("matches Verdict");
        assert_eq!(answer, Verdict { approved: true });
    }

    #[test]
    fn task_answer_rejects_another_shape() {
        let err = <AgentTask<Verdict> as AgentStep>::answer(step_output(json!(["approved"])))
            .expect_err("not a Verdict");
        assert!(matches!(err, EngineError::Serialization(_)));
    }

    #[test]
    fn retry_policy_and_resume_settings_are_applied() {
        let mut labels = BTreeMap::new();
        labels.insert("team".to_string(), "qa".to_string());
        let config = Bare
            .task("p".to_string())
            .retry_policy(RetryPolicy::new(2))
            .resume("sess")
            .session_id("sid")
            .resume_prompt("go on")
            .resume_environment("env-1")
            .verbose(true)
            .pod_labels(labels.clone())
            .into_config();
        assert_eq!(config.retry.map(|r| r.max_retries()), Some(2));
        assert_eq!(config.resume_session_id.as_deref(), Some("sess"));
        assert_eq!(config.session_id.as_deref(), Some("sid"));
        assert_eq!(config.resume_prompt.as_deref(), Some("go on"));
        assert_eq!(config.resume_environment_id.as_deref(), Some("env-1"));
        assert!(config.verbose);
        assert_eq!(config.pod_labels, labels);
    }

    #[test]
    fn clone_keeps_the_config() {
        let task = Bare
            .task("p".to_string())
            .model("haiku")
            .pod_label("a", "b");
        let copy = task.clone();
        assert_eq!(
            to_value(task.into_config()).expect("serialize"),
            to_value(copy.into_config()).expect("serialize")
        );
    }
}
