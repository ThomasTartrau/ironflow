//! Execution plans -- what a run *would* do, without doing it.
//!
//! Ironflow workflows are Rust-native handlers, not declarative graphs: the
//! only way to know which steps a run would create is to execute the handler
//! with every step method short-circuited. That is what *plan mode* is.
//!
//! A recorder is attached to a
//! [`WorkflowContext`](crate::context::WorkflowContext) by
//! [`Engine::plan_handler`](crate::engine::Engine::plan_handler). Every step
//! entry point checks it first, records a [`PlannedStep`], and returns a
//! synthetic [`StepOutput`] without touching the store, the provider, the
//! event bus or the network.
//!
//! # The success-shaped output assumption
//!
//! Synthetic outputs are shaped like a *successful* step (`exit_code: 0`,
//! `status: 200`), so a native `if build.is_success()` branch in the handler
//! follows the happy path. A plan therefore shows the nominal branch, not
//! every branch the run might take. Conditions declared with
//! [`WorkflowContext::when`](crate::context::WorkflowContext::when) are
//! evaluated against the run input and reported; those declared with
//! [`WorkflowContext::when_dynamic`](crate::context::WorkflowContext::when_dynamic)
//! are reported as [`ConditionResult::Unevaluable`].
//!
//! # Why the global dry-run flag is not used
//!
//! [`ironflow_core::dry_run`] exposes a process-wide switch. Plan mode does not
//! flip it: nothing is executed while planning, so the flag would buy nothing,
//! and flipping a process-wide flag would corrupt real runs executing
//! concurrently in the same process.

use std::collections::HashMap;
use std::mem::replace;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use ironflow_store::entities::{RunFilter, RunStatus, StepKind, StepStatus};
use ironflow_store::store::Store;

use crate::config::StepConfig;
use crate::error::EngineError;
use crate::executor::{StepArtifacts, StepOutput};

/// How deep sub-workflows are expanded when the caller does not say.
pub const DEFAULT_PLAN_MAX_DEPTH: u32 = 3;

/// Hard cap on the number of steps a single plan may record.
///
/// A handler that loops forever would otherwise plan forever. Once the cap is
/// reached the plan is returned truncated.
pub const MAX_PLANNED_STEPS: usize = 1000;

/// How many past runs are sampled to estimate step durations.
pub const DEFAULT_ESTIMATE_SAMPLE_RUNS: u32 = 20;

/// Outcome of a branch condition as seen by the planner.
///
/// # Examples
///
/// ```
/// use serde_json::{Error, to_value};
///
/// use ironflow_engine::plan::ConditionResult;
///
/// # fn example() -> Result<(), Error> {
/// let condition = ConditionResult::Evaluated {
///     expression: "production run".to_string(),
///     value: true,
/// };
/// assert_eq!(to_value(&condition)?["state"], "evaluated");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConditionResult {
    /// Resolved against the run input.
    Evaluated {
        /// Label the handler gave the branch. A name for the operator, never
        /// parsed nor evaluated.
        expression: String,
        /// What the predicate returned for this input.
        value: bool,
    },
    /// The step is explicitly skipped (recorded by
    /// [`WorkflowContext::skip`](crate::context::WorkflowContext::skip)).
    Skipped {
        /// Reason the handler gave for skipping.
        reason: String,
    },
    /// Depends on a previous step's output; unknown before the run.
    Unevaluable {
        /// Label the handler gave the branch.
        expression: String,
        /// Why the planner cannot resolve it.
        reason: String,
    },
}

/// One step the planner expects the run to create.
///
/// # Examples
///
/// ```
/// use ironflow_engine::plan::PlannedStep;
/// use ironflow_store::entities::StepKind;
///
/// let step = PlannedStep {
///     name: "build".to_string(),
///     kind: StepKind::Shell,
///     workflow: "deploy".to_string(),
///     depth: 0,
///     depends_on: Vec::new(),
///     condition: None,
///     parallel_group: None,
///     estimated_duration: None,
/// };
/// assert_eq!(step.name, "build");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedStep {
    /// Step name as the handler declares it.
    pub name: String,
    /// Kind of operation this step performs.
    pub kind: StepKind,
    /// Workflow that owns this step (top-level name, or the sub-workflow's).
    pub workflow: String,
    /// Sub-workflow nesting depth; `0` for the top-level workflow.
    pub depth: u32,
    /// Names of the steps this one runs after.
    ///
    /// Names, not identifiers: the same name can appear twice when two
    /// branches or two sub-workflows declare a step with the same name.
    /// Consumers must key on position, not on name.
    pub depends_on: Vec<String>,
    /// Branch condition recorded just before this step, if the handler
    /// declared one.
    ///
    /// A condition pending before a parallel wave attaches to the *first*
    /// member of the wave only.
    pub condition: Option<ConditionResult>,
    /// Parallel wave this step belongs to, when it runs concurrently with
    /// its siblings.
    pub parallel_group: Option<String>,
    /// Average duration of this step across past completed runs.
    #[serde(with = "opt_duration_ms")]
    pub estimated_duration: Option<Duration>,
}

/// The full plan for one workflow and one input payload.
///
/// # Examples
///
/// ```
/// use ironflow_engine::plan::ExecutionPlan;
///
/// let plan = ExecutionPlan {
///     workflow: "deploy".to_string(),
///     steps: Vec::new(),
///     estimated_duration: None,
///     max_depth: 3,
///     truncated: false,
///     incomplete_reason: None,
/// };
/// assert!(plan.steps.is_empty());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    /// Workflow the plan was built for.
    pub workflow: String,
    /// Steps the run is expected to create, in execution order.
    pub steps: Vec<PlannedStep>,
    /// Sum of the step estimates, counting each parallel wave once.
    #[serde(with = "opt_duration_ms")]
    pub estimated_duration: Option<Duration>,
    /// Sub-workflow expansion depth used for this plan.
    pub max_depth: u32,
    /// `true` when the step cap or the depth limit cut the plan short.
    pub truncated: bool,
    /// Why the plan stopped early (handler error, cap, depth limit).
    pub incomplete_reason: Option<String>,
}

/// Knobs for [`Engine::plan_handler`](crate::engine::Engine::plan_handler).
///
/// # Examples
///
/// ```
/// use ironflow_engine::plan::{PlanOptions, DEFAULT_PLAN_MAX_DEPTH};
///
/// let options = PlanOptions::default();
/// assert_eq!(options.max_depth, DEFAULT_PLAN_MAX_DEPTH);
/// ```
#[derive(Debug, Clone)]
pub struct PlanOptions {
    /// How deep sub-workflows are expanded. Must be at least 1.
    pub max_depth: u32,
    /// Whether step durations are estimated from run history.
    pub estimate_durations: bool,
    /// How many past runs are sampled when estimating durations.
    pub sample_runs: u32,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_PLAN_MAX_DEPTH,
            estimate_durations: true,
            sample_runs: DEFAULT_ESTIMATE_SAMPLE_RUNS,
        }
    }
}

/// Serde adapter mapping `Option<Duration>` to an optional millisecond count.
mod opt_duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Serialize a duration as whole milliseconds.
    pub(super) fn serialize<S: Serializer>(
        value: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let millis = value.map(|d| d.as_millis() as u64);
        millis.serialize(serializer)
    }

    /// Deserialize whole milliseconds back into a duration.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        let millis = Option::<u64>::deserialize(deserializer)?;
        Ok(millis.map(Duration::from_millis))
    }
}

/// A [`PlanRecorder`] shared by a context and every child context it spawns.
pub(crate) type SharedPlanRecorder = Arc<Mutex<PlanRecorder>>;

/// Accumulates the steps a handler declares while running in plan mode.
pub(crate) struct PlanRecorder {
    payload: Value,
    workflow: String,
    steps: Vec<PlannedStep>,
    last_names: Vec<String>,
    pending_condition: Option<ConditionResult>,
    depth: u32,
    max_depth: u32,
    parallel_groups: u32,
    estimates: HashMap<String, Duration>,
    truncated: bool,
    incomplete_reason: Option<String>,
}

impl PlanRecorder {
    /// Create a recorder for one workflow and one input payload.
    pub(crate) fn new(
        workflow: String,
        payload: Value,
        max_depth: u32,
        estimates: HashMap<String, Duration>,
    ) -> Self {
        Self {
            payload,
            workflow,
            steps: Vec::new(),
            last_names: Vec::new(),
            pending_condition: None,
            depth: 0,
            max_depth,
            parallel_groups: 0,
            estimates,
            truncated: false,
            incomplete_reason: None,
        }
    }

    /// The payload the plan is being computed for.
    pub(crate) fn payload(&self) -> Value {
        self.payload.clone()
    }

    /// Swap the payload, returning the previous one.
    ///
    /// A sub-workflow plans against its own payload; the parent's is restored
    /// when the expansion returns.
    pub(crate) fn swap_payload(&mut self, next: Value) -> Value {
        replace(&mut self.payload, next)
    }

    /// Attach a condition to the next recorded step.
    pub(crate) fn set_condition(&mut self, condition: ConditionResult) {
        self.pending_condition = Some(condition);
    }

    /// The historical estimate for a step name, if any.
    pub(crate) fn estimate_for(&self, name: &str) -> Option<Duration> {
        self.estimates.get(name).copied()
    }

    /// Provide a fallback estimate for a step the history does not cover.
    ///
    /// Used by steps whose duration is declared rather than observed, such as
    /// a delay. History always wins when it exists.
    pub(crate) fn seed_estimate(&mut self, name: &str, duration: Duration) {
        self.estimates.entry(name.to_string()).or_insert(duration);
    }

    /// Record a step, returning `false` when the step cap is already reached.
    ///
    /// Does not update the dependency frontier: a parallel wave sets every
    /// member at once, so the caller decides via [`set_last`](Self::set_last).
    pub(crate) fn record(
        &mut self,
        name: &str,
        kind: StepKind,
        workflow: &str,
        parallel_group: Option<String>,
    ) -> bool {
        if self.steps.len() >= MAX_PLANNED_STEPS {
            self.truncated = true;
            if self.incomplete_reason.is_none() {
                self.incomplete_reason = Some(format!("step cap of {MAX_PLANNED_STEPS} reached"));
            }
            return false;
        }

        let estimated_duration = self.estimates.get(name).copied();
        self.steps.push(PlannedStep {
            name: name.to_string(),
            kind,
            workflow: workflow.to_string(),
            depth: self.depth,
            depends_on: self.last_names.clone(),
            condition: self.pending_condition.take(),
            parallel_group,
            estimated_duration,
        });
        true
    }

    /// Set the dependency frontier the next recorded step depends on.
    pub(crate) fn set_last(&mut self, names: Vec<String>) {
        self.last_names = names;
    }

    /// Allocate a fresh parallel group name.
    pub(crate) fn next_group(&mut self) -> String {
        self.parallel_groups += 1;
        format!("parallel-{}", self.parallel_groups)
    }

    /// Enter a sub-workflow, unless that would cross the depth limit.
    pub(crate) fn enter_workflow(&mut self) -> bool {
        if self.depth + 1 > self.max_depth {
            self.truncated = true;
            if self.incomplete_reason.is_none() {
                self.incomplete_reason = Some(format!(
                    "sub-workflow expansion stopped at depth {}",
                    self.max_depth
                ));
            }
            return false;
        }
        self.depth += 1;
        true
    }

    /// Leave the current sub-workflow.
    pub(crate) fn leave_workflow(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Record why the plan stopped early. The first writer wins.
    pub(crate) fn fail(&mut self, reason: String) {
        self.truncated = true;
        if self.incomplete_reason.is_none() {
            self.incomplete_reason = Some(reason);
        }
    }

    /// Build the plan without consuming the recorder.
    pub(crate) fn snapshot(&self) -> ExecutionPlan {
        ExecutionPlan {
            workflow: self.workflow.clone(),
            estimated_duration: total_estimate(&self.steps),
            steps: self.steps.clone(),
            max_depth: self.max_depth,
            truncated: self.truncated,
            incomplete_reason: self.incomplete_reason.clone(),
        }
    }

    /// Consume the recorder and build the plan.
    pub(crate) fn into_plan(self) -> ExecutionPlan {
        ExecutionPlan {
            workflow: self.workflow,
            estimated_duration: total_estimate(&self.steps),
            steps: self.steps,
            max_depth: self.max_depth,
            truncated: self.truncated,
            incomplete_reason: self.incomplete_reason,
        }
    }
}

/// Lock a shared recorder, ignoring poisoning.
///
/// A panicking handler must not turn every later plan into a panic of its own.
/// Never hold the returned guard across an `.await`.
pub(crate) fn lock_plan(plan: &SharedPlanRecorder) -> MutexGuard<'_, PlanRecorder> {
    plan.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Sum of sequential estimates; each parallel group counts once, at its
/// slowest member.
///
/// Returns `None` when no step carries an estimate, so an absent history is
/// reported as "unknown" rather than as zero.
fn total_estimate(steps: &[PlannedStep]) -> Option<Duration> {
    let mut total = Duration::ZERO;
    let mut seen_any = false;
    let mut current_group: Option<&str> = None;
    let mut group_max = Duration::ZERO;

    for step in steps {
        match step.parallel_group.as_deref() {
            Some(group) if current_group == Some(group) => {
                if let Some(estimate) = step.estimated_duration {
                    seen_any = true;
                    group_max = group_max.max(estimate);
                }
            }
            Some(group) => {
                if current_group.is_some() {
                    total += group_max;
                }
                current_group = Some(group);
                group_max = step.estimated_duration.unwrap_or(Duration::ZERO);
                if step.estimated_duration.is_some() {
                    seen_any = true;
                }
            }
            None => {
                if current_group.is_some() {
                    total += group_max;
                    current_group = None;
                    group_max = Duration::ZERO;
                }
                if let Some(estimate) = step.estimated_duration {
                    seen_any = true;
                    total += estimate;
                }
            }
        }
    }

    if current_group.is_some() {
        total += group_max;
    }

    seen_any.then_some(total)
}

/// Synthetic, success-shaped output returned to the handler in plan mode.
///
/// Shaped so that `output.is_success()` is `true` for shell and HTTP steps:
/// the plan follows the branch a successful run would take.
pub(crate) fn planned_output(config: &StepConfig, estimate: Option<Duration>) -> StepOutput {
    let output = match config {
        StepConfig::Shell(_) => json!({"stdout": "", "stderr": "", "exit_code": 0}),
        StepConfig::Http(_) => json!({"status": 200, "headers": {}, "body": ""}),
        StepConfig::Agent(_) => json!({}),
        StepConfig::Workflow(c) => json!({
            "run_id": Value::Null,
            "workflow_name": c.workflow_name,
            "status": "completed",
            "cost_usd": 0,
            "duration_ms": 0,
        }),
        StepConfig::Approval(_) | StepConfig::Decision(_) | StepConfig::Delay(_) => Value::Null,
    };

    StepOutput {
        output,
        duration_ms: estimate.map(|d| d.as_millis() as u64).unwrap_or(0),
        cost_usd: Decimal::ZERO,
        input_tokens: None,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        output_tokens: None,
        model: None,
        debug_messages: None,
        artifacts: StepArtifacts::default(),
    }
}

/// Synthetic output for a custom operation step in plan mode.
pub(crate) fn planned_custom_output(estimate: Option<Duration>) -> StepOutput {
    StepOutput {
        output: json!({}),
        duration_ms: estimate.map(|d| d.as_millis() as u64).unwrap_or(0),
        cost_usd: Decimal::ZERO,
        input_tokens: None,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        output_tokens: None,
        model: None,
        debug_messages: None,
        artifacts: StepArtifacts::default(),
    }
}

/// Average completed-step durations for a workflow, keyed by step name.
///
/// Samples the most recent completed runs of `workflow_name` and averages the
/// duration of every completed step, per name. Step names absent from the
/// history are simply absent from the map.
///
/// # Errors
///
/// Returns [`EngineError::Store`] when the history query fails.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
///
/// use ironflow_engine::error::EngineError;
/// use ironflow_engine::plan::estimate_durations;
/// use ironflow_store::store::Store;
///
/// # async fn example(store: &Arc<dyn Store>) -> Result<(), EngineError> {
/// let estimates = estimate_durations(store, "deploy", 20).await?;
/// println!("{} steps have a history", estimates.len());
/// # Ok(())
/// # }
/// ```
pub async fn estimate_durations(
    store: &Arc<dyn Store>,
    workflow_name: &str,
    sample_runs: u32,
) -> Result<HashMap<String, Duration>, EngineError> {
    let page = store
        .list_runs(
            RunFilter {
                workflow_name: Some(workflow_name.to_string()),
                status: Some(RunStatus::Completed),
                has_steps: Some(true),
                ..RunFilter::default()
            },
            1,
            sample_runs.clamp(1, 100),
        )
        .await?;

    let mut totals: HashMap<String, (u64, u64)> = HashMap::new();
    for run in &page.items {
        for step in store.list_steps(run.id).await? {
            if step.status.state != StepStatus::Completed {
                continue;
            }
            let entry = totals.entry(step.name).or_insert((0, 0));
            entry.0 += step.duration_ms;
            entry.1 += 1;
        }
    }

    Ok(totals
        .into_iter()
        .filter(|(_, (_, count))| *count > 0)
        .map(|(name, (sum, count))| (name, Duration::from_millis(sum / count)))
        .collect())
}

impl ExecutionPlan {
    /// Total estimate in whole milliseconds, when the plan has one.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use ironflow_engine::plan::ExecutionPlan;
    ///
    /// let plan = ExecutionPlan {
    ///     workflow: "deploy".to_string(),
    ///     steps: Vec::new(),
    ///     estimated_duration: Some(Duration::from_millis(1500)),
    ///     max_depth: 3,
    ///     truncated: false,
    ///     incomplete_reason: None,
    /// };
    /// assert_eq!(plan.estimated_duration_ms(), Some(1500));
    /// ```
    pub fn estimated_duration_ms(&self) -> Option<u64> {
        self.estimated_duration.map(|d| d.as_millis() as u64)
    }
}

impl PlannedStep {
    /// This step's estimate in whole milliseconds, when it has one.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use ironflow_engine::plan::PlannedStep;
    /// use ironflow_store::entities::StepKind;
    ///
    /// let step = PlannedStep {
    ///     name: "build".to_string(),
    ///     kind: StepKind::Shell,
    ///     workflow: "deploy".to_string(),
    ///     depth: 0,
    ///     depends_on: Vec::new(),
    ///     condition: None,
    ///     parallel_group: None,
    ///     estimated_duration: Some(Duration::from_millis(250)),
    /// };
    /// assert_eq!(step.estimated_duration_ms(), Some(250));
    /// ```
    pub fn estimated_duration_ms(&self) -> Option<u64> {
        self.estimated_duration.map(|d| d.as_millis() as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::thread::spawn;
    use std::time::Duration;

    use serde_json::{from_value, json, to_value};

    use crate::config::{HttpConfig, ShellConfig};

    use super::*;

    fn step(name: &str, group: Option<&str>, estimate: Option<u64>) -> PlannedStep {
        PlannedStep {
            name: name.to_string(),
            kind: StepKind::Shell,
            workflow: "wf".to_string(),
            depth: 0,
            depends_on: Vec::new(),
            condition: None,
            parallel_group: group.map(str::to_string),
            estimated_duration: estimate.map(Duration::from_millis),
        }
    }

    #[test]
    fn total_estimate_sums_sequential_steps() {
        let steps = vec![
            step("a", None, Some(100)),
            step("b", None, Some(250)),
            step("c", None, Some(50)),
        ];
        assert_eq!(total_estimate(&steps), Some(Duration::from_millis(400)));
    }

    #[test]
    fn total_estimate_counts_a_parallel_group_once_at_its_slowest() {
        let steps = vec![
            step("build", None, Some(100)),
            step("t1", Some("parallel-1"), Some(300)),
            step("t2", Some("parallel-1"), Some(700)),
            step("t3", Some("parallel-1"), Some(200)),
            step("deploy", None, Some(100)),
        ];
        assert_eq!(total_estimate(&steps), Some(Duration::from_millis(900)));
    }

    #[test]
    fn total_estimate_handles_a_trailing_parallel_group() {
        let steps = vec![
            step("build", None, Some(100)),
            step("t1", Some("parallel-1"), Some(300)),
            step("t2", Some("parallel-1"), Some(700)),
        ];
        assert_eq!(total_estimate(&steps), Some(Duration::from_millis(800)));
    }

    #[test]
    fn total_estimate_is_none_without_any_estimate() {
        let steps = vec![step("a", None, None), step("b", None, None)];
        assert_eq!(total_estimate(&steps), None);
    }

    #[test]
    fn planned_step_duration_round_trips_as_milliseconds() {
        let original = step("a", None, Some(1234));
        let value = to_value(&original).expect("serialize");
        assert_eq!(value["estimated_duration"], 1234);

        let back: PlannedStep = from_value(value).expect("deserialize");
        assert_eq!(back.estimated_duration, Some(Duration::from_millis(1234)));
    }

    #[test]
    fn planned_step_duration_round_trips_when_absent() {
        let original = step("a", None, None);
        let value = to_value(&original).expect("serialize");
        assert!(value["estimated_duration"].is_null());

        let back: PlannedStep = from_value(value).expect("deserialize");
        assert_eq!(back.estimated_duration, None);
    }

    #[test]
    fn planned_shell_and_http_outputs_look_successful() {
        let shell = planned_output(&StepConfig::Shell(ShellConfig::new("echo hi")), None);
        assert!(shell.is_success());

        let http = planned_output(
            &StepConfig::Http(HttpConfig::get("https://example.com")),
            None,
        );
        assert!(http.is_success());
    }

    #[test]
    fn planned_output_carries_the_estimate_as_its_duration() {
        let output = planned_output(
            &StepConfig::Shell(ShellConfig::new("echo hi")),
            Some(Duration::from_millis(900)),
        );
        assert_eq!(output.duration_ms, 900);
        assert_eq!(output.cost_usd, Decimal::ZERO);
    }

    #[test]
    fn planned_custom_output_is_an_empty_object() {
        let output = planned_custom_output(None);
        assert_eq!(output.output, json!({}));
        assert_eq!(output.duration_ms, 0);
    }

    #[test]
    fn condition_result_serializes_its_state_tag() {
        let evaluated = to_value(ConditionResult::Evaluated {
            expression: "env == prod".to_string(),
            value: false,
        })
        .expect("serialize");
        assert_eq!(evaluated["state"], "evaluated");
        assert_eq!(evaluated["value"], false);

        let skipped = to_value(ConditionResult::Skipped {
            reason: "not prod".to_string(),
        })
        .expect("serialize");
        assert_eq!(skipped["state"], "skipped");

        let unevaluable = to_value(ConditionResult::Unevaluable {
            expression: "build succeeded".to_string(),
            reason: "depends on a step output".to_string(),
        })
        .expect("serialize");
        assert_eq!(unevaluable["state"], "unevaluable");
    }

    #[test]
    fn recorder_records_dependencies_and_conditions() {
        let mut recorder = PlanRecorder::new("wf".to_string(), json!({}), 3, HashMap::new());
        assert!(recorder.record("a", StepKind::Shell, "wf", None));
        recorder.set_last(vec!["a".to_string()]);
        recorder.set_condition(ConditionResult::Skipped {
            reason: "nope".to_string(),
        });
        assert!(recorder.record("b", StepKind::Shell, "wf", None));

        let plan = recorder.into_plan();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[1].depends_on, vec!["a".to_string()]);
        assert!(matches!(
            plan.steps[1].condition,
            Some(ConditionResult::Skipped { .. })
        ));
        assert!(plan.steps[0].condition.is_none());
    }

    #[test]
    fn recorder_stops_at_the_step_cap() {
        let mut recorder = PlanRecorder::new("wf".to_string(), json!({}), 3, HashMap::new());
        for index in 0..MAX_PLANNED_STEPS {
            assert!(recorder.record(&format!("s{index}"), StepKind::Shell, "wf", None));
        }
        assert!(!recorder.record("overflow", StepKind::Shell, "wf", None));

        let plan = recorder.into_plan();
        assert_eq!(plan.steps.len(), MAX_PLANNED_STEPS);
        assert!(plan.truncated);
        assert!(
            plan.incomplete_reason
                .expect("a reason")
                .contains("step cap")
        );
    }

    #[test]
    fn recorder_refuses_to_expand_past_the_depth_limit() {
        let mut recorder = PlanRecorder::new("wf".to_string(), json!({}), 1, HashMap::new());
        assert!(recorder.enter_workflow());
        assert!(!recorder.enter_workflow());

        let plan = recorder.snapshot();
        assert!(plan.truncated);
        assert!(
            plan.incomplete_reason
                .expect("a reason")
                .contains("depth 1")
        );
    }

    #[test]
    fn recorder_allocates_successive_parallel_group_names() {
        let mut recorder = PlanRecorder::new("wf".to_string(), json!({}), 3, HashMap::new());
        assert_eq!(recorder.next_group(), "parallel-1");
        assert_eq!(recorder.next_group(), "parallel-2");
    }

    #[test]
    fn recorder_swaps_and_restores_the_payload() {
        let mut recorder =
            PlanRecorder::new("wf".to_string(), json!({"env": "prod"}), 3, HashMap::new());
        let previous = recorder.swap_payload(json!({"env": "dev"}));
        assert_eq!(previous, json!({"env": "prod"}));
        assert_eq!(recorder.payload(), json!({"env": "dev"}));
        recorder.swap_payload(previous);
        assert_eq!(recorder.payload(), json!({"env": "prod"}));
    }

    #[test]
    fn recorder_keeps_the_first_failure_reason() {
        let mut recorder = PlanRecorder::new("wf".to_string(), json!({}), 3, HashMap::new());
        recorder.fail("first".to_string());
        recorder.fail("second".to_string());
        let plan = recorder.into_plan();
        assert_eq!(plan.incomplete_reason.as_deref(), Some("first"));
        assert!(plan.truncated);
    }

    #[test]
    fn recorder_uses_the_history_estimate_for_a_known_step() {
        let estimates = HashMap::from([("build".to_string(), Duration::from_millis(400))]);
        let mut recorder = PlanRecorder::new("wf".to_string(), json!({}), 3, estimates);
        assert_eq!(
            recorder.estimate_for("build"),
            Some(Duration::from_millis(400))
        );
        assert_eq!(recorder.estimate_for("unknown"), None);
        recorder.record("build", StepKind::Shell, "wf", None);
        let plan = recorder.into_plan();
        assert_eq!(plan.estimated_duration, Some(Duration::from_millis(400)));
    }

    #[test]
    fn lock_plan_recovers_from_poisoning() {
        let shared: SharedPlanRecorder = Arc::new(Mutex::new(PlanRecorder::new(
            "wf".to_string(),
            json!({}),
            3,
            HashMap::new(),
        )));
        let poisoner = Arc::clone(&shared);
        let _ = spawn(move || {
            let _guard = poisoner.lock().expect("lock");
            panic!("poison the mutex");
        })
        .join();

        assert_eq!(lock_plan(&shared).payload(), json!({}));
    }
}
