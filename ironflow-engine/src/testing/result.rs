//! What a [`TestEngine`](crate::testing::TestEngine) run leaves behind.
//!
//! [`TestResult`] and [`TestStep`] read back the run and the steps the engine
//! persisted, so an assertion sees exactly what the API and the dashboard would
//! serve for that run.

use std::time::Duration;

use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;

use ironflow_store::models::{Run, RunStatus, Step, StepKind, StepStatus};

use crate::executor::StepResult;

/// Stand-in for a step that recorded no input, and for a run with no steps.
static NULL: Value = Value::Null;

/// One persisted step, with assertion-friendly accessors.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::testing::TestResult;
/// use ironflow_store::models::StepStatus;
///
/// # fn example(result: &TestResult) {
/// let build = result.step("build");
/// assert_eq!(build.status(), StepStatus::Completed);
/// assert_eq!(build.output()["exit_code"], 0);
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct TestStep {
    step: Step,
    output: Value,
}

impl TestStep {
    /// Wrap a persisted step, defaulting a missing output to [`Value::Null`].
    pub(crate) fn new(step: Step) -> Self {
        let output = step.output.clone().unwrap_or(Value::Null);
        Self { step, output }
    }

    /// The step name, as passed to the context method that created it.
    pub fn name(&self) -> &str {
        &self.step.name
    }

    /// The kind of operation this step ran.
    pub fn kind(&self) -> &StepKind {
        &self.step.kind
    }

    /// The terminal status the step reached.
    pub fn status(&self) -> StepStatus {
        self.step.status.state
    }

    /// Persisted output, [`Value::Null`] when the step produced none.
    pub fn output(&self) -> &Value {
        &self.output
    }

    /// Persisted input, the serialized step config.
    pub fn input(&self) -> &Value {
        self.step.input.as_ref().unwrap_or(&NULL)
    }

    /// Error message recorded on the step, if it failed or was rejected.
    pub fn error(&self) -> Option<&str> {
        self.step.error.as_deref()
    }

    /// Wall-clock duration of the step.
    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.step.duration_ms)
    }

    /// Cost charged by the step, in USD. Zero for everything but agent steps.
    pub fn cost_usd(&self) -> Decimal {
        self.step.cost_usd
    }

    /// Whether the step reached [`StepStatus::Completed`].
    pub fn is_completed(&self) -> bool {
        self.status() == StepStatus::Completed
    }

    /// Whether the step came from an
    /// [`on_error`](crate::context::WorkflowContext::on_error) handler.
    pub fn is_error_handler(&self) -> bool {
        self.step.is_error_handler
    }

    /// The underlying store record, for assertions the accessors do not cover.
    pub fn raw(&self) -> &Step {
        &self.step
    }
}

/// Outcome of a [`TestEngine`](crate::testing::TestEngine) run.
///
/// A handler that fails is *not* an error here: the run is read back with
/// [`RunStatus::Failed`] and [`error`](Self::error) set. Only wiring failures
/// (unknown handler, duplicate handler name, store error) surface as an `Err`
/// from [`TestEngine::run`](crate::testing::TestEngine::run).
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::testing::TestResult;
/// use ironflow_store::models::RunStatus;
///
/// # fn example(result: &TestResult) {
/// assert_eq!(result.status(), RunStatus::Completed);
/// assert_eq!(result.step_names(), vec!["build", "test", "deploy"]);
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct TestResult {
    run: Run,
    steps: Vec<TestStep>,
    step_results: Vec<StepResult>,
    error: Option<String>,
}

impl TestResult {
    /// Assemble a result from what the store holds after execution.
    pub(crate) fn new(
        run: Run,
        steps: Vec<Step>,
        step_results: Vec<StepResult>,
        error: Option<String>,
    ) -> Self {
        Self {
            run,
            steps: steps.into_iter().map(TestStep::new).collect(),
            step_results,
            error,
        }
    }

    /// The status the run finished in.
    pub fn status(&self) -> RunStatus {
        self.run.status.state
    }

    /// The persisted run record.
    pub fn run(&self) -> &Run {
        &self.run
    }

    /// The run identifier, for [`TestEngine::resume`](crate::testing::TestEngine::resume).
    pub fn run_id(&self) -> Uuid {
        self.run.id
    }

    /// Every persisted step, ordered by position.
    pub fn steps(&self) -> &[TestStep] {
        &self.steps
    }

    /// The step names, ordered by position.
    pub fn step_names(&self) -> Vec<&str> {
        self.steps.iter().map(TestStep::name).collect()
    }

    /// The first step carrying `name`.
    ///
    /// Steps of a parallel wave share a position, and a handler may reuse a
    /// name: disambiguate those with [`steps`](Self::steps).
    ///
    /// # Panics
    ///
    /// Panics when no step carries that name; the message lists the names that
    /// exist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::testing::TestResult;
    ///
    /// # fn example(result: &TestResult) {
    /// assert!(result.step("deploy").is_completed());
    /// # }
    /// ```
    pub fn step(&self, name: &str) -> &TestStep {
        self.try_step(name).unwrap_or_else(|| {
            panic!(
                "no step named {name:?} in this run; steps are {:?}",
                self.step_names()
            )
        })
    }

    /// The first step carrying `name`, or `None`.
    pub fn try_step(&self, name: &str) -> Option<&TestStep> {
        self.steps.iter().find(|step| step.name() == name)
    }

    /// Output of the last persisted step, [`Value::Null`] when the run has none.
    pub fn output(&self) -> &Value {
        self.steps.last().map_or(&NULL, TestStep::output)
    }

    /// Wall-clock duration recorded on the run.
    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.run.duration_ms)
    }

    /// Total cost of the run, in USD.
    pub fn cost_usd(&self) -> Decimal {
        self.run.cost_usd
    }

    /// Why the run stopped, if it did not complete.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Per-step metrics as the engine reported them.
    ///
    /// Empty when the run failed: the engine returns the error instead of a
    /// [`WorkflowResult`](crate::engine::WorkflowResult) on that path. Use
    /// [`steps`](Self::steps), which always reflects the store.
    pub fn step_results(&self) -> &[StepResult] {
        &self.step_results
    }

    /// Whether the run reached [`RunStatus::Completed`].
    pub fn is_completed(&self) -> bool {
        self.status() == RunStatus::Completed
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use chrono::Utc;
    use serde_json::json;

    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, NewStep, StepUpdate, TriggerKind, step_trace_id};
    use ironflow_store::store::RunStore;

    use super::*;

    /// Build a two-step run in a store: `build` completed, `deploy` failed.
    ///
    /// `Run` and `Step` are `#[non_exhaustive]`, so they can only be obtained
    /// from a store.
    async fn persisted_run() -> (Run, Vec<Step>) {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "deploy".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run();

        for (position, name) in [(0u32, "build"), (1, "deploy")] {
            let step = store
                .create_step(NewStep {
                    run_id: run.id,
                    trace_id: step_trace_id(run.id, name, position),
                    name: name.to_string(),
                    kind: StepKind::Shell,
                    position,
                    input: Some(json!({"type": "shell", "command": name})),
                    is_error_handler: false,
                })
                .await
                .expect("create step");

            store
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Running),
                        started_at: Some(Utc::now()),
                        ..StepUpdate::default()
                    },
                )
                .await
                .expect("start step");

            let update = if name == "build" {
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(json!({"stdout": "built", "stderr": "", "exit_code": 0})),
                    duration_ms: Some(12),
                    completed_at: Some(Utc::now()),
                    ..StepUpdate::default()
                }
            } else {
                StepUpdate {
                    status: Some(StepStatus::Failed),
                    error: Some("boom".to_string()),
                    completed_at: Some(Utc::now()),
                    ..StepUpdate::default()
                }
            };
            store
                .update_step(step.id, update)
                .await
                .expect("finish the step");
        }

        let steps = store.list_steps(run.id).await.expect("list steps");
        let run = store
            .get_run(run.id)
            .await
            .expect("get run")
            .expect("the run exists");
        (run, steps)
    }

    #[tokio::test]
    async fn accessors_read_back_what_was_persisted() {
        let (run, steps) = persisted_run().await;
        let result = TestResult::new(run, steps, Vec::new(), Some("boom".to_string()));

        assert_eq!(result.step_names(), vec!["build", "deploy"]);
        assert_eq!(result.error(), Some("boom"));
        assert!(result.step_results().is_empty());
        assert!(!result.is_completed());

        let build = result.step("build");
        assert!(build.is_completed());
        assert_eq!(build.output()["stdout"], "built");
        assert_eq!(build.input()["command"], "build");
        assert_eq!(build.duration(), Duration::from_millis(12));
        assert_eq!(build.cost_usd(), Decimal::ZERO);
        assert_eq!(build.kind(), &StepKind::Shell);
        assert!(!build.is_error_handler());
        assert_eq!(build.raw().name, "build");
        assert!(build.error().is_none());
    }

    #[tokio::test]
    async fn output_is_the_last_step_output() {
        let (run, steps) = persisted_run().await;
        let result = TestResult::new(run, steps, Vec::new(), None);

        // `deploy` failed without an output.
        assert_eq!(result.output(), &Value::Null);
        assert_eq!(result.step("deploy").status(), StepStatus::Failed);
        assert_eq!(result.step("deploy").error(), Some("boom"));
    }

    #[tokio::test]
    async fn run_level_accessors_reflect_the_store() {
        let (run, steps) = persisted_run().await;
        let run_id = run.id;
        let result = TestResult::new(run, steps, Vec::new(), None);

        assert_eq!(result.run_id(), run_id);
        assert_eq!(result.run().workflow_name, "deploy");
        assert_eq!(result.status(), RunStatus::Pending);
        assert_eq!(result.duration(), Duration::ZERO);
        assert_eq!(result.cost_usd(), Decimal::ZERO);
        assert_eq!(result.steps().len(), 2);
    }

    #[tokio::test]
    async fn try_step_returns_none_for_an_unknown_name() {
        let (run, steps) = persisted_run().await;
        let result = TestResult::new(run, steps, Vec::new(), None);

        assert!(result.try_step("nope").is_none());
    }

    #[tokio::test]
    #[should_panic(expected = "no step named \"nope\"")]
    async fn step_panics_for_an_unknown_name() {
        let (run, steps) = persisted_run().await;
        let result = TestResult::new(run, steps, Vec::new(), None);

        result.step("nope");
    }

    #[tokio::test]
    async fn a_run_without_steps_has_a_null_output() {
        let (run, _) = persisted_run().await;
        let result = TestResult::new(run, Vec::new(), Vec::new(), None);

        assert_eq!(result.output(), &Value::Null);
        assert!(result.step_names().is_empty());
    }
}
