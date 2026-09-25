//! [`SubWorkflowOutput`] -- what a parent gets back from a sub-workflow.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ironflow_store::entities::RunStatus;

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
        }
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

    /// Final status of the child run: `Completed`, or `Warning` when one of its
    /// `allow_failure` steps failed.
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
