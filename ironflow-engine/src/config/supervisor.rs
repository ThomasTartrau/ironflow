//! Agent supervisor delegating tasks to the roles of its team.
//!
//! A supervisor is an [`AgentRole`] whose output is a [`Turn`]: each round it
//! either gives its answer or delegates tasks to its [`Team`]. The team is an
//! enum written by the workflow author: one variant per role the supervisor
//! may delegate to, mapped to that role by [`Team::assign`]. A role missing
//! from the enum cannot be a target.
//!
//! `ctx.supervise` runs the rounds: each delegated task becomes a child run of
//! the [`TeamWorkflow`](crate::handler::TeamWorkflow) of the team, and the
//! next round gets the tasks and their results as untrusted content.

use std::fmt;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Error as JsonError, Value};

use super::agent::{AgentStep, AgentStepConfig};
use super::agent_role::AgentTask;
use super::approval::ApprovalConfig;

/// Prefix of the reason recorded on the skipped step of a refused delegation.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::DELEGATION_REFUSED;
///
/// assert_eq!(DELEGATION_REFUSED, "DELEGATION_REFUSED");
/// ```
pub const DELEGATION_REFUSED: &str = "DELEGATION_REFUSED";

/// Longest supervisor name `ctx.supervise` accepts, in bytes.
///
/// The name prefixes the step of every round and every delegated task; with a
/// role name of [`MAX_ROLE_NAME_LEN`](super::MAX_ROLE_NAME_LEN) bytes and round
/// and task numbers of up to 9 digits, the step name still fits the 255
/// characters of the store.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::MAX_SUPERVISOR_NAME_LEN;
///
/// assert_eq!(MAX_SUPERVISOR_NAME_LEN, 100);
/// ```
pub const MAX_SUPERVISOR_NAME_LEN: usize = 100;

/// What a supervisor answers each round: tasks to delegate, or its answer.
///
/// Use it as the [`Output`](super::AgentRole::Output) of the supervisor role:
/// `type Output = Turn<Plan, Squad>`. The root of the JSON Schema sent to the
/// provider is an object; the variants of the team are a `oneOf`, which the
/// Claude CLI accepts but the strict structured output of OpenAI does not.
///
/// Exactly one of the two fields is set: a turn with both, or with neither,
/// fails the run.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::Turn;
/// use serde_json::{from_value, json};
///
/// let turn: Turn<String, String> = from_value(json!({"answer": "done"}))?;
/// assert_eq!(turn.answer.as_deref(), Some("done"));
/// assert!(turn.delegate.is_empty());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
// Without it, `#[serde(default)]` makes serde require `A: Default` and `T: Default`.
#[serde(bound(deserialize = "A: Deserialize<'de>, T: Deserialize<'de>"))]
pub struct Turn<A, T> {
    /// Tasks to delegate this round, one per team member to call. Leave it
    /// empty to give your answer instead.
    #[serde(default)]
    pub delegate: Vec<T>,
    /// Your final answer. Set it only when you delegate nothing.
    #[serde(default)]
    pub answer: Option<A>,
}

/// The roles a supervisor may delegate to, declared as an enum.
///
/// Each variant is a task for one role and carries its input. The exhaustive
/// `match` of [`assign`](Self::assign) is the declaration: the supervisor can
/// only delegate to a role named there. Its delegated tasks run in child runs
/// of [`TeamWorkflow<Self>`](crate::handler::TeamWorkflow), registered under
/// [`NAME`](Self::NAME) next to the supervisor's workflow.
///
/// The input of a variant is written by the supervisor agent: a target role
/// should treat it as untrusted, for instance with
/// [`Prompt::untrusted`](super::Prompt::untrusted).
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{AgentRole, Assignment, Prompt, Team};
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
///
/// struct Coder;
///
/// impl AgentRole for Coder {
///     type Input = String;
///     type Output = String;
///     fn prompt(&self, input: String) -> String {
///         Prompt::new().text("Implement:").untrusted(&input).to_string()
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
///
///     fn assign(self) -> Assignment {
///         match self {
///             Squad::Coder(input) => Coder.task(input).into(),
///         }
///     }
/// }
///
/// let assignment = Squad::Coder("the login".to_string()).assign();
/// assert_eq!(assignment.role(), "Coder");
/// ```
///
/// A role the enum does not name cannot be a target:
///
/// ```compile_fail,E0599
/// # use ironflow_engine::config::{Assignment, Team};
/// # use schemars::JsonSchema;
/// # use serde::{Deserialize, Serialize};
/// #[derive(Serialize, Deserialize, JsonSchema)]
/// enum Squad {
///     Coder(String),
/// }
/// # impl Team for Squad {
/// #     const NAME: &'static str = "squad";
/// #     fn assign(self) -> Assignment { unimplemented!() }
/// # }
/// let _ = Squad::Deployer("prod".to_string());
/// ```
pub trait Team: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static {
    /// Name of the [`TeamWorkflow`](crate::handler::TeamWorkflow) of this
    /// team: the workflow name of every child run it starts.
    const NAME: &'static str;

    /// The role task of this delegation.
    fn assign(self) -> Assignment;
}

/// The task of a role, as [`Team::assign`] gives it to the child run.
///
/// Built only from an [`AgentTask`] (`Role.task(input).into()`): a team runs
/// the roles it names, never an agent config of its own.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{AgentRole, ApprovalConfig, Assignment};
///
/// struct Deployer;
///
/// impl AgentRole for Deployer {
///     type Input = String;
///     type Output = String;
///     fn prompt(&self, input: String) -> String {
///         format!("Deploy {input}")
///     }
/// }
///
/// let assignment = Assignment::from(Deployer.task("v2".to_string()))
///     .after_approval(ApprovalConfig::new("Let the agent deploy v2?"));
/// assert_eq!(assignment.role(), "Deployer");
/// assert!(assignment.approval().is_some());
/// ```
#[derive(Clone)]
pub struct Assignment {
    config: AgentStepConfig,
    role: String,
    approval: Option<ApprovalConfig>,
    check_answer: AnswerCheck,
}

impl fmt::Debug for Assignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Assignment")
            .field("role", &self.role)
            .field("config", &self.config)
            .field("approval", &self.approval)
            .finish()
    }
}

impl Assignment {
    /// Gate the task behind a human approval: the child run suspends on it
    /// before the agent starts, and a rejection fails the child run.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, ApprovalConfig, Assignment};
    /// # struct Deployer;
    /// # impl AgentRole for Deployer {
    /// #     type Input = String;
    /// #     type Output = String;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// let assignment = Assignment::from(Deployer.task("v2".to_string()))
    ///     .after_approval(ApprovalConfig::new("Deploy v2?"));
    /// assert!(assignment.approval().is_some());
    /// ```
    pub fn after_approval(mut self, approval: ApprovalConfig) -> Self {
        self.approval = Some(approval);
        self
    }

    /// Name of the role the task runs, as [`AgentRole::name`](super::AgentRole::name)
    /// gives it.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, Assignment};
    /// # struct Coder;
    /// # impl AgentRole for Coder {
    /// #     type Input = String;
    /// #     type Output = String;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// assert_eq!(Assignment::from(Coder.task("x".to_string())).role(), "Coder");
    /// ```
    pub fn role(&self) -> &str {
        &self.role
    }

    /// The approval the task waits for, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ironflow_engine::config::{AgentRole, Assignment};
    /// # struct Coder;
    /// # impl AgentRole for Coder {
    /// #     type Input = String;
    /// #     type Output = String;
    /// #     fn prompt(&self, input: String) -> String { input }
    /// # }
    /// assert!(Assignment::from(Coder.task("x".to_string())).approval().is_none());
    /// ```
    pub fn approval(&self) -> Option<&ApprovalConfig> {
        self.approval.as_ref()
    }

    /// Check that an answer of the agent is an `Output` of the role.
    pub(crate) fn answer_check(&self) -> AnswerCheck {
        self.check_answer
    }

    /// The agent config, the role, and the approval of the task.
    pub(crate) fn into_parts(self) -> (AgentStepConfig, String, Option<ApprovalConfig>) {
        (self.config, self.role, self.approval)
    }
}

/// Checks that a value deserializes into the output type of a role.
pub(crate) type AnswerCheck = fn(&Value) -> Result<(), JsonError>;

/// Whether `answer` is a `T`, the output type the role was built with.
fn answer_is<T: DeserializeOwned>(answer: &Value) -> Result<(), JsonError> {
    T::deserialize(answer).map(|_| ())
}

impl<T: DeserializeOwned> From<AgentTask<T>> for Assignment {
    fn from(task: AgentTask<T>) -> Self {
        let config = task.into_config();
        let role = config
            .role
            .clone()
            .expect("AgentTask::from_role always records the role name");
        Self {
            config,
            role,
            approval: None,
            check_answer: answer_is::<T>,
        }
    }
}

/// Limits of a supervision: how many rounds, and how many tasks per round.
///
/// Without a workflow guard or a cost cap on the run, nothing else bounds a
/// supervisor that keeps delegating. A round over the task limit, or a
/// supervisor still delegating after the last round, fails the run.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::SupervisionLimits;
///
/// let limits = SupervisionLimits::default().max_rounds(3).max_tasks_per_round(2);
/// assert_eq!(limits.rounds(), 3);
/// assert_eq!(limits.tasks_per_round(), 2);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupervisionLimits {
    max_rounds: u32,
    max_tasks_per_round: u32,
}

impl Default for SupervisionLimits {
    /// 5 rounds of at most 5 tasks.
    fn default() -> Self {
        Self {
            max_rounds: 5,
            max_tasks_per_round: 5,
        }
    }
}

impl SupervisionLimits {
    /// Set the number of supervisor rounds, the last one included.
    ///
    /// # Panics
    ///
    /// Panics if `rounds` is 0.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::SupervisionLimits;
    ///
    /// assert_eq!(SupervisionLimits::default().max_rounds(8).rounds(), 8);
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_engine::config::SupervisionLimits;
    ///
    /// SupervisionLimits::default().max_rounds(0);
    /// ```
    pub fn max_rounds(mut self, rounds: u32) -> Self {
        assert!(rounds > 0, "a supervision needs at least one round");
        self.max_rounds = rounds;
        self
    }

    /// Set how many tasks the supervisor may delegate in one round.
    ///
    /// # Panics
    ///
    /// Panics if `tasks` is 0.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::SupervisionLimits;
    ///
    /// assert_eq!(SupervisionLimits::default().max_tasks_per_round(1).tasks_per_round(), 1);
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_engine::config::SupervisionLimits;
    ///
    /// SupervisionLimits::default().max_tasks_per_round(0);
    /// ```
    pub fn max_tasks_per_round(mut self, tasks: u32) -> Self {
        assert!(tasks > 0, "a supervision round needs room for one task");
        self.max_tasks_per_round = tasks;
        self
    }

    /// The number of rounds.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::SupervisionLimits;
    ///
    /// assert_eq!(SupervisionLimits::default().rounds(), 5);
    /// ```
    pub fn rounds(&self) -> u32 {
        self.max_rounds
    }

    /// The number of tasks allowed per round.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::SupervisionLimits;
    ///
    /// assert_eq!(SupervisionLimits::default().tasks_per_round(), 5);
    /// ```
    pub fn tasks_per_round(&self) -> u32 {
        self.max_tasks_per_round
    }
}

#[cfg(test)]
mod tests {
    use schemars::schema_for;
    use serde_json::{Value, from_value, json, to_value};

    use super::*;
    use crate::config::AgentRole;

    struct Coder;

    impl AgentRole for Coder {
        type Input = String;
        type Output = String;

        fn prompt(&self, input: String) -> String {
            format!("Implement: {input}")
        }
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    enum Squad {
        Coder(String),
    }

    #[test]
    fn turn_schema_is_an_object_at_the_root() {
        let schema = to_value(schema_for!(Turn<String, Squad>)).expect("schema");
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["delegate"].is_object());
        assert!(schema["properties"]["answer"].is_object());
    }

    #[test]
    fn turn_reads_tasks_without_an_answer() {
        let turn: Turn<String, Value> =
            from_value(json!({"delegate": [{"Coder": "login"}]})).expect("turn");
        assert_eq!(turn.delegate, vec![json!({"Coder": "login"})]);
        assert_eq!(turn.answer, None);
    }

    #[test]
    fn empty_turn_reads_as_neither() {
        let turn: Turn<String, Value> = from_value(json!({})).expect("turn");
        assert!(turn.delegate.is_empty());
        assert_eq!(turn.answer, None);
    }

    #[test]
    fn assignment_keeps_the_role_config() {
        let (config, role, approval) = Assignment::from(Coder.task("login".to_string()))
            .after_approval(ApprovalConfig::new("ok?"))
            .into_parts();
        assert_eq!(role, "Coder");
        assert_eq!(config.prompt, "Implement: login");
        assert_eq!(config.role.as_deref(), Some("Coder"));
        assert!(config.json_schema.is_some());
        assert!(approval.is_some());
    }

    #[test]
    fn default_limits() {
        let limits = SupervisionLimits::default();
        assert_eq!(limits.rounds(), 5);
        assert_eq!(limits.tasks_per_round(), 5);
    }

    #[test]
    #[should_panic(expected = "at least one round")]
    fn zero_rounds_panics() {
        let _ = SupervisionLimits::default().max_rounds(0);
    }

    #[test]
    #[should_panic(expected = "room for one task")]
    fn zero_tasks_panics() {
        let _ = SupervisionLimits::default().max_tasks_per_round(0);
    }
}
