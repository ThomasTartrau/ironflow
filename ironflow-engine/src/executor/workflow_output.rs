//! [`SubWorkflowOutput`] -- what a parent gets back from a sub-workflow.
//!
//! [`SubWorkflowOutcome`] adds the case of a sub-workflow started with a
//! concurrency key that another active run already holds.

use std::fmt;

use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_value};
use uuid::Uuid;

use ironflow_store::entities::RunStatus;

use crate::error::EngineError;

/// Result of a [`workflow`](crate::context::WorkflowContext::workflow) step.
///
/// While planning, no child run is created: [`run_id`](Self::run_id) is
/// [`Uuid::nil`] and the metrics are zero.
///
/// It is also the persisted output of the step, so a stored workflow step
/// reads back with [`StepOutput::json`](super::StepOutput::json).
///
/// # Examples
///
/// ```
/// use ironflow_engine::executor::SubWorkflowOutput;
/// use ironflow_store::entities::RunStatus;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let run_id = Uuid::now_v7();
/// let output = SubWorkflowOutput::new(run_id, "collect", RunStatus::Completed, Decimal::ZERO, 1200);
/// assert_eq!(output.run_id(), run_id);
/// assert_eq!(output.workflow_name(), "collect");
/// assert_eq!(output.status(), RunStatus::Completed);
/// assert_eq!(output.duration_ms(), 1200);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubWorkflowOutput {
    run_id: Uuid,
    workflow_name: String,
    status: RunStatus,
    cost_usd: Decimal,
    duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<Value>,
}

impl SubWorkflowOutput {
    /// Assemble the result of a child run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::SubWorkflowOutput;
    /// use ironflow_store::entities::RunStatus;
    /// use rust_decimal::Decimal;
    /// use uuid::Uuid;
    ///
    /// let output = SubWorkflowOutput::new(Uuid::nil(), "collect", RunStatus::Warning, Decimal::ONE, 0);
    /// assert_eq!(output.cost_usd(), Decimal::ONE);
    /// ```
    pub fn new(
        run_id: Uuid,
        workflow_name: &str,
        status: RunStatus,
        cost_usd: Decimal,
        duration_ms: u64,
    ) -> Self {
        Self {
            run_id,
            workflow_name: workflow_name.to_string(),
            status,
            cost_usd,
            duration_ms,
            error: None,
            output: None,
        }
    }

    /// Attach the error of a child run tolerated by `allow_failure`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::SubWorkflowOutput;
    /// use ironflow_store::entities::RunStatus;
    /// use rust_decimal::Decimal;
    /// use uuid::Uuid;
    ///
    /// let output = SubWorkflowOutput::new(Uuid::nil(), "collect", RunStatus::Failed, Decimal::ZERO, 0)
    ///     .with_error("boom");
    /// assert_eq!(output.error(), Some("boom"));
    /// ```
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Attach the output the child handler set with
    /// [`set_output`](crate::context::WorkflowContext::set_output).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::SubWorkflowOutput;
    /// use ironflow_store::entities::RunStatus;
    /// use rust_decimal::Decimal;
    /// use serde_json::json;
    /// use uuid::Uuid;
    ///
    /// # use ironflow_engine::error::EngineError;
    /// # fn example() -> Result<(), EngineError> {
    /// let output = SubWorkflowOutput::new(Uuid::nil(), "review", RunStatus::Completed, Decimal::ZERO, 0)
    ///     .with_output(Some(json!(true)));
    /// assert_eq!(output.output::<bool>()?, Some(true));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_output(mut self, output: Option<Value>) -> Self {
        self.output = output;
        self
    }

    /// The child run, to read its steps from the store. [`Uuid::nil`] while
    /// planning.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::SubWorkflowOutput;
    /// use ironflow_store::entities::RunStatus;
    /// use rust_decimal::Decimal;
    /// use uuid::Uuid;
    ///
    /// let output = SubWorkflowOutput::new(Uuid::nil(), "collect", RunStatus::Completed, Decimal::ZERO, 0);
    /// assert!(output.run_id().is_nil());
    /// ```
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    /// Name of the child workflow.
    pub fn workflow_name(&self) -> &str {
        &self.workflow_name
    }

    /// Final status of the child run: `Completed`, `Warning` when one of its
    /// `allow_failure` steps failed, or (only when the step was started with
    /// `allow_failure`) `Failed` / `Cancelled`.
    pub fn status(&self) -> RunStatus {
        self.status
    }

    /// Cost of the child run, in USD. Already included in the parent's cost.
    pub fn cost_usd(&self) -> Decimal {
        self.cost_usd
    }

    /// Wall-clock duration of the child run, in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Error of a failed or cancelled child run tolerated by `allow_failure`,
    /// `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::SubWorkflowOutput;
    /// use ironflow_store::entities::RunStatus;
    /// use rust_decimal::Decimal;
    /// use uuid::Uuid;
    ///
    /// let output = SubWorkflowOutput::new(Uuid::nil(), "collect", RunStatus::Completed, Decimal::ZERO, 0);
    /// assert_eq!(output.error(), None);
    /// ```
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The typed output the child handler set with
    /// [`set_output`](crate::context::WorkflowContext::set_output).
    ///
    /// `Ok(None)` when the child set no output. A failed child tolerated by
    /// `allow_failure` keeps the output it set before failing. On replay the
    /// value is read from the recorded step, not from the child run.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Serialization`] if the output does not
    /// deserialize into `T`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::SubWorkflowOutput;
    /// use ironflow_store::entities::RunStatus;
    /// use rust_decimal::Decimal;
    /// use serde::Deserialize;
    /// use serde_json::json;
    /// use uuid::Uuid;
    ///
    /// #[derive(Deserialize)]
    /// struct Review {
    ///     approved: bool,
    /// }
    ///
    /// # use ironflow_engine::error::EngineError;
    /// # fn example() -> Result<(), EngineError> {
    /// let child = SubWorkflowOutput::new(Uuid::nil(), "review", RunStatus::Completed, Decimal::ZERO, 0)
    ///     .with_output(Some(json!({"approved": true})));
    /// let review: Option<Review> = child.output()?;
    /// assert!(review.is_some_and(|r| r.approved));
    /// # Ok(())
    /// # }
    /// ```
    pub fn output<T: DeserializeOwned>(&self) -> Result<Option<T>, EngineError> {
        self.output
            .as_ref()
            .map(|value| from_value(value.clone()))
            .transpose()
            .map_err(EngineError::Serialization)
    }
}

/// A sub-workflow skipped because its concurrency key is held by another
/// active run.
///
/// Persisted as the step output `{"concurrency_conflict": {"key": .., "run_id": ..}}`
/// and replayed as-is on resume.
///
/// # Examples
///
/// ```
/// use ironflow_engine::executor::ConcurrencyConflict;
/// use uuid::Uuid;
///
/// let holder = Uuid::now_v7();
/// let conflict = ConcurrencyConflict::new("issue:12", holder);
/// assert_eq!(conflict.key(), "issue:12");
/// assert_eq!(conflict.run_id(), holder);
/// assert!(conflict.to_string().contains("issue:12"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConcurrencyConflict {
    key: String,
    run_id: Uuid,
}

impl ConcurrencyConflict {
    /// Record that `run_id` holds `key`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::ConcurrencyConflict;
    /// use uuid::Uuid;
    ///
    /// let conflict = ConcurrencyConflict::new("deploy:prod", Uuid::nil());
    /// assert_eq!(conflict.key(), "deploy:prod");
    /// ```
    pub fn new(key: impl Into<String>, run_id: Uuid) -> Self {
        Self {
            key: key.into(),
            run_id,
        }
    }

    /// The contested concurrency key.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::ConcurrencyConflict;
    /// use uuid::Uuid;
    ///
    /// assert_eq!(ConcurrencyConflict::new("k", Uuid::nil()).key(), "k");
    /// ```
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The active run holding the key, at the time of the conflict.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::ConcurrencyConflict;
    /// use uuid::Uuid;
    ///
    /// let holder = Uuid::now_v7();
    /// assert_eq!(ConcurrencyConflict::new("k", holder).run_id(), holder);
    /// ```
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }
}

impl fmt::Display for ConcurrencyConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "conflict on concurrency key {:?} (run {})",
            self.key, self.run_id
        )
    }
}

/// Result of a
/// [`workflow_with`](crate::context::WorkflowContext::workflow_with) step.
///
/// # Examples
///
/// ```
/// use ironflow_engine::executor::{ConcurrencyConflict, SubWorkflowOutcome};
/// use uuid::Uuid;
///
/// let outcome = SubWorkflowOutcome::Conflict(ConcurrencyConflict::new("issue:12", Uuid::nil()));
/// match &outcome {
///     SubWorkflowOutcome::Completed(output) => println!("child run {}", output.run_id()),
///     SubWorkflowOutcome::Conflict(conflict) => println!("held by {}", conflict.run_id()),
/// }
/// assert!(outcome.output().is_none());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum SubWorkflowOutcome {
    /// The child run was created and finished.
    Completed(SubWorkflowOutput),
    /// No child run was created: another active run holds the concurrency key.
    Conflict(ConcurrencyConflict),
}

impl SubWorkflowOutcome {
    /// The conflict, when no child run was created.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::{ConcurrencyConflict, SubWorkflowOutcome};
    /// use uuid::Uuid;
    ///
    /// let outcome = SubWorkflowOutcome::Conflict(ConcurrencyConflict::new("k", Uuid::nil()));
    /// assert_eq!(outcome.conflict().map(|c| c.key()), Some("k"));
    /// ```
    pub fn conflict(&self) -> Option<&ConcurrencyConflict> {
        match self {
            SubWorkflowOutcome::Conflict(conflict) => Some(conflict),
            SubWorkflowOutcome::Completed(_) => None,
        }
    }

    /// The child result, when the child run was created and finished.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::{SubWorkflowOutcome, SubWorkflowOutput};
    /// use ironflow_store::entities::RunStatus;
    /// use rust_decimal::Decimal;
    /// use uuid::Uuid;
    ///
    /// let output = SubWorkflowOutput::new(Uuid::nil(), "collect", RunStatus::Completed, Decimal::ZERO, 0);
    /// let outcome = SubWorkflowOutcome::Completed(output.clone());
    /// assert_eq!(outcome.output(), Some(&output));
    /// assert!(outcome.conflict().is_none());
    /// ```
    pub fn output(&self) -> Option<&SubWorkflowOutput> {
        match self {
            SubWorkflowOutcome::Completed(output) => Some(output),
            SubWorkflowOutcome::Conflict(_) => None,
        }
    }
}

/// The persisted output of a completed `Workflow` step, as read back on replay.
///
/// `Conflict` is tried first: a flat [`SubWorkflowOutput`] has no
/// `concurrency_conflict` field, so outputs recorded before concurrency keys
/// existed still read back as `Completed`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum RecordedWorkflowStep {
    /// The step was skipped on a concurrency conflict.
    Conflict {
        /// The recorded conflict.
        concurrency_conflict: ConcurrencyConflict,
    },
    /// The child run finished.
    Completed(SubWorkflowOutput),
}

impl From<RecordedWorkflowStep> for SubWorkflowOutcome {
    fn from(recorded: RecordedWorkflowStep) -> Self {
        match recorded {
            RecordedWorkflowStep::Conflict {
                concurrency_conflict,
            } => SubWorkflowOutcome::Conflict(concurrency_conflict),
            RecordedWorkflowStep::Completed(output) => SubWorkflowOutcome::Completed(output),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;

    #[test]
    fn serializes_to_the_persisted_step_output() {
        let run_id = Uuid::now_v7();
        let output = SubWorkflowOutput::new(
            run_id,
            "collect",
            RunStatus::Warning,
            Decimal::new(25, 2),
            1200,
        );

        assert_eq!(
            to_value(&output).expect("serialize"),
            json!({
                "run_id": run_id,
                "workflow_name": "collect",
                "status": "warning",
                "cost_usd": 0.25,
                "duration_ms": 1200,
            })
        );
    }

    #[test]
    fn a_persisted_step_output_reads_back() {
        let run_id = Uuid::now_v7();
        let stored = json!({
            "run_id": run_id,
            "workflow_name": "collect",
            "status": "completed",
            "cost_usd": 0,
            "duration_ms": 7,
        });

        let output: SubWorkflowOutput = from_value(stored).expect("deserialize");

        assert_eq!(output.run_id(), run_id);
        assert_eq!(output.status(), RunStatus::Completed);
        assert_eq!(output.cost_usd(), Decimal::ZERO);
        assert_eq!(output.duration_ms(), 7);
    }

    #[test]
    fn a_recorded_conflict_round_trips() {
        let holder = Uuid::now_v7();
        let conflict = ConcurrencyConflict::new("issue:12", holder);
        let stored = json!({ "concurrency_conflict": conflict });

        assert_eq!(
            stored,
            json!({ "concurrency_conflict": { "key": "issue:12", "run_id": holder } })
        );

        let recorded: RecordedWorkflowStep = from_value(stored).expect("deserialize");
        let outcome = SubWorkflowOutcome::from(recorded);
        assert_eq!(outcome, SubWorkflowOutcome::Conflict(conflict));
        assert_eq!(
            outcome.conflict().map(ConcurrencyConflict::run_id),
            Some(holder)
        );
        assert!(outcome.output().is_none());
    }

    #[test]
    fn a_flat_output_still_reads_back_as_completed() {
        let run_id = Uuid::now_v7();
        let stored = json!({
            "run_id": run_id,
            "workflow_name": "collect",
            "status": "completed",
            "cost_usd": 0,
            "duration_ms": 7,
        });

        let recorded: RecordedWorkflowStep = from_value(stored).expect("deserialize");
        let outcome = SubWorkflowOutcome::from(recorded);
        let output = outcome.output().expect("completed outcome");
        assert_eq!(output.run_id(), run_id);
        assert_eq!(output.duration_ms(), 7);
        assert!(outcome.conflict().is_none());
    }

    #[test]
    fn conflict_display_names_the_key_and_the_holder() {
        let holder = Uuid::now_v7();
        let text = ConcurrencyConflict::new("issue:12", holder).to_string();
        assert!(text.contains("\"issue:12\""));
        assert!(text.contains(&holder.to_string()));
    }

    #[test]
    fn an_error_roundtrips() {
        let output = SubWorkflowOutput::new(
            Uuid::now_v7(),
            "collect",
            RunStatus::Failed,
            Decimal::ZERO,
            3,
        )
        .with_error("boom");

        let back: SubWorkflowOutput =
            from_value(to_value(&output).expect("serialize")).expect("deserialize");

        assert_eq!(back, output);
        assert_eq!(back.error(), Some("boom"));
    }

    #[test]
    fn a_legacy_output_without_error_reads_back_as_none() {
        let output: SubWorkflowOutput = from_value(json!({
            "run_id": Uuid::now_v7(),
            "workflow_name": "collect",
            "status": "completed",
            "cost_usd": 0,
            "duration_ms": 7,
        }))
        .expect("deserialize");

        assert_eq!(output.error(), None);
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Verdict {
        approved: bool,
        score: u8,
    }

    fn child() -> SubWorkflowOutput {
        SubWorkflowOutput::new(
            Uuid::now_v7(),
            "review",
            RunStatus::Completed,
            Decimal::ZERO,
            5,
        )
    }

    #[test]
    fn set_output_none_reads_back_as_none() {
        let output: Option<Verdict> = child().output().expect("no output");
        assert_eq!(output, None);
    }

    #[test]
    fn set_output_typed_value_reads_back() {
        let output = child().with_output(Some(json!({"approved": true, "score": 7})));

        let verdict: Option<Verdict> = output.output().expect("deserialize");
        assert_eq!(
            verdict,
            Some(Verdict {
                approved: true,
                score: 7
            })
        );
    }

    #[test]
    fn set_output_of_the_wrong_type_is_a_serialization_error() {
        let output = child().with_output(Some(json!({"approved": "yes"})));

        let result = output.output::<Verdict>();
        assert!(matches!(result, Err(EngineError::Serialization(_))));
    }

    #[test]
    fn set_output_round_trips_through_the_step_output() {
        let output = child().with_output(Some(json!({"approved": false, "score": 1})));

        let stored = to_value(&output).expect("serialize");
        assert_eq!(stored["output"], json!({"approved": false, "score": 1}));

        let back: SubWorkflowOutput = from_value(stored.clone()).expect("deserialize");
        assert_eq!(back, output);

        let recorded: RecordedWorkflowStep = from_value(stored).expect("deserialize");
        let outcome = SubWorkflowOutcome::from(recorded);
        let replayed = outcome.output().expect("completed outcome");
        assert_eq!(
            replayed.output::<Verdict>().expect("deserialize"),
            Some(Verdict {
                approved: false,
                score: 1
            })
        );
    }

    #[test]
    fn set_output_absent_is_not_serialized() {
        let stored = to_value(child()).expect("serialize");
        assert!(stored.get("output").is_none());
    }

    #[test]
    fn set_output_legacy_step_output_without_the_field_reads_back_as_none() {
        let output: SubWorkflowOutput = from_value(json!({
            "run_id": Uuid::now_v7(),
            "workflow_name": "collect",
            "status": "completed",
            "cost_usd": 0,
            "duration_ms": 7,
        }))
        .expect("deserialize");

        assert_eq!(output.output::<Verdict>().expect("no output"), None);
    }

    #[test]
    fn an_output_without_run_id_is_refused() {
        let result = from_value::<SubWorkflowOutput>(json!({
            "workflow_name": "collect",
            "status": "completed",
            "cost_usd": 0,
            "duration_ms": 0,
        }));
        assert!(result.is_err());
    }
}
