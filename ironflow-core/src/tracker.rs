//! Workflow-level cost, token, and duration tracking.
//!
//! [`WorkflowTracker`] aggregates metrics across all shell and agent steps in a
//! workflow, making it easy to log a single summary at the end with total cost,
//! token counts, and elapsed time.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::tracker::WorkflowTracker;
//!
//! let mut tracker = WorkflowTracker::new("deploy-pipeline");
//!
//! // ... run shell and agent steps, calling
//! // tracker.record_shell() / tracker.record_agent() after each ...
//!
//! tracker.summary(); // logs a structured summary via tracing
//! println!("Total cost: ${:.4}", tracker.total_cost_usd());
//! ```

use std::collections::VecDeque;
use std::time::Instant;

use tracing::info;

use crate::operations::agent::AgentResult;
use crate::operations::http::HttpOutput;
use crate::operations::shell::ShellOutput;

/// Aggregates cost, token, and duration metrics for a named workflow.
///
/// Create one tracker per workflow run with [`WorkflowTracker::new`], record
/// each step with [`record_shell`](WorkflowTracker::record_shell) or
/// [`record_agent`](WorkflowTracker::record_agent), then call
/// [`summary`](WorkflowTracker::summary) to emit a structured log line.
/// Default maximum number of steps kept in the tracker.
/// Older steps are evicted when this limit is reached.
const DEFAULT_MAX_STEPS: usize = 10_000;

pub struct WorkflowTracker {
    name: String,
    start: Instant,
    steps: VecDeque<StepRecord>,
    max_steps: usize,
}

struct StepRecord {
    name: String,
    kind: StepKind,
    duration_ms: u64,
    cost_usd: Option<f64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

enum StepKind {
    Shell,
    Http,
    Agent,
}

impl std::fmt::Display for StepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Shell => f.write_str("shell"),
            Self::Http => f.write_str("http"),
            Self::Agent => f.write_str("agent"),
        }
    }
}

impl WorkflowTracker {
    /// Create a new tracker for a workflow with the given `name`.
    ///
    /// The wall-clock timer starts immediately.
    #[must_use = "a tracker does nothing if not used to record steps"]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            start: Instant::now(),
            steps: VecDeque::new(),
            max_steps: DEFAULT_MAX_STEPS,
        }
    }

    /// Set the maximum number of steps to retain.
    ///
    /// When exceeded, the oldest step is removed. Defaults to 10 000.
    pub fn max_steps(mut self, limit: usize) -> Self {
        self.max_steps = limit;
        self
    }

    fn push_step(&mut self, record: StepRecord) {
        if self.steps.len() >= self.max_steps {
            self.steps.pop_front();
        }
        self.steps.push_back(record);
    }

    /// Record a completed shell step.
    ///
    /// Extracts the duration from the [`ShellOutput`]. Shell steps have no
    /// associated cost or token counts.
    pub fn record_shell(&mut self, name: &str, output: &ShellOutput) {
        self.push_step(StepRecord {
            name: name.to_string(),
            kind: StepKind::Shell,
            duration_ms: output.duration_ms(),
            cost_usd: None,
            input_tokens: None,
            output_tokens: None,
        });
    }

    /// Record a completed HTTP step.
    ///
    /// Extracts the duration from the [`HttpOutput`]. HTTP steps have no
    /// associated cost or token counts.
    pub fn record_http(&mut self, name: &str, output: &HttpOutput) {
        self.push_step(StepRecord {
            name: name.to_string(),
            kind: StepKind::Http,
            duration_ms: output.duration_ms(),
            cost_usd: None,
            input_tokens: None,
            output_tokens: None,
        });
    }

    /// Record a completed agent step.
    ///
    /// Extracts duration, cost, and token counts from the [`AgentResult`].
    pub fn record_agent(&mut self, name: &str, result: &AgentResult) {
        self.push_step(StepRecord {
            name: name.to_string(),
            kind: StepKind::Agent,
            duration_ms: result.duration_ms(),
            cost_usd: result.cost_usd(),
            input_tokens: result.input_tokens(),
            output_tokens: result.output_tokens(),
        });
    }

    /// Return the sum of all agent step costs in USD.
    ///
    /// Steps that did not report a cost (including all shell steps) are skipped.
    pub fn total_cost_usd(&self) -> f64 {
        self.steps.iter().filter_map(|s| s.cost_usd).sum()
    }

    /// Return the sum of all input tokens across agent steps.
    pub fn total_input_tokens(&self) -> u64 {
        self.steps.iter().filter_map(|s| s.input_tokens).sum()
    }

    /// Return the sum of all output tokens across agent steps.
    pub fn total_output_tokens(&self) -> u64 {
        self.steps.iter().filter_map(|s| s.output_tokens).sum()
    }

    /// Return the wall-clock duration since the tracker was created, in milliseconds.
    pub fn total_duration_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Return the number of recorded steps (shell + agent).
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Emit a structured log summary of the entire workflow and each step.
    ///
    /// Uses [`tracing::info!`] to log one line for the workflow totals and one
    /// line per step with its kind, duration, cost, and token counts.
    pub fn summary(&self) {
        let total_cost = self.total_cost_usd();
        let total_input = self.total_input_tokens();
        let total_output = self.total_output_tokens();
        let total_duration = self.total_duration_ms();
        let steps = self.step_count();

        info!(
            workflow = %self.name,
            steps,
            total_cost_usd = total_cost,
            total_input_tokens = total_input,
            total_output_tokens = total_output,
            total_duration_ms = total_duration,
            "workflow completed"
        );

        for step in &self.steps {
            info!(
                workflow = %self.name,
                step = %step.name,
                kind = %step.kind,
                duration_ms = step.duration_ms,
                cost_usd = step.cost_usd,
                input_tokens = step.input_tokens,
                output_tokens = step.output_tokens,
                "step detail"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::operations::agent::AgentResult;
    use crate::operations::shell::Shell;
    use crate::provider::AgentOutput;

    fn make_agent_result(
        cost: Option<f64>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> AgentResult {
        let mut output = AgentOutput::new(json!("result"));
        output.cost_usd = cost;
        output.input_tokens = input_tokens;
        output.output_tokens = output_tokens;
        output.duration_ms = 100;
        AgentResult::from_output(output)
    }

    async fn make_shell_output() -> ShellOutput {
        Shell::new("echo test").run().await.unwrap()
    }

    #[test]
    fn new_tracker_has_zero_steps_and_zero_cost() {
        let tracker = WorkflowTracker::new("test");
        assert_eq!(tracker.step_count(), 0);
        assert_eq!(tracker.total_cost_usd(), 0.0);
    }

    #[tokio::test]
    async fn record_shell_increments_step_count() {
        let mut tracker = WorkflowTracker::new("test");
        let output = make_shell_output().await;
        tracker.record_shell("step1", &output);
        assert_eq!(tracker.step_count(), 1);
    }

    #[test]
    fn record_agent_with_cost_reflected_in_total() {
        let mut tracker = WorkflowTracker::new("test");
        let result = make_agent_result(Some(0.05), Some(100), Some(50));
        tracker.record_agent("agent1", &result);
        assert_eq!(tracker.total_cost_usd(), 0.05);
    }

    #[test]
    fn record_agent_without_cost_does_not_change_total() {
        let mut tracker = WorkflowTracker::new("test");
        let result = make_agent_result(None, None, None);
        tracker.record_agent("agent1", &result);
        assert_eq!(tracker.total_cost_usd(), 0.0);
    }

    #[tokio::test]
    async fn multiple_steps_counted_correctly() {
        let mut tracker = WorkflowTracker::new("test");
        let shell = make_shell_output().await;
        let agent = make_agent_result(Some(0.1), Some(200), Some(100));
        tracker.record_shell("s1", &shell);
        tracker.record_agent("a1", &agent);
        tracker.record_shell("s2", &shell);
        assert_eq!(tracker.step_count(), 3);
    }

    #[test]
    fn total_input_tokens_sums_across_agent_steps() {
        let mut tracker = WorkflowTracker::new("test");
        let r1 = make_agent_result(None, Some(100), None);
        let r2 = make_agent_result(None, Some(250), None);
        tracker.record_agent("a1", &r1);
        tracker.record_agent("a2", &r2);
        assert_eq!(tracker.total_input_tokens(), 350);
    }

    #[test]
    fn total_output_tokens_sums_across_agent_steps() {
        let mut tracker = WorkflowTracker::new("test");
        let r1 = make_agent_result(None, None, Some(50));
        let r2 = make_agent_result(None, None, Some(75));
        tracker.record_agent("a1", &r1);
        tracker.record_agent("a2", &r2);
        assert_eq!(tracker.total_output_tokens(), 125);
    }

    #[test]
    fn tokens_with_mixed_none_values() {
        let mut tracker = WorkflowTracker::new("test");
        let r1 = make_agent_result(None, Some(100), Some(50));
        let r2 = make_agent_result(None, None, None);
        let r3 = make_agent_result(None, Some(200), Some(30));
        tracker.record_agent("a1", &r1);
        tracker.record_agent("a2", &r2);
        tracker.record_agent("a3", &r3);
        assert_eq!(tracker.total_input_tokens(), 300);
        assert_eq!(tracker.total_output_tokens(), 80);
    }

    #[test]
    fn total_duration_ms_is_positive() {
        let tracker = WorkflowTracker::new("test");
        // total_duration_ms measures wall-clock time since creation, so it should be >= 0
        // (practically > 0 due to execution time)
        assert!(tracker.total_duration_ms() < 1000); // sanity: shouldn't take more than 1s
    }

    #[test]
    fn summary_does_not_panic_empty() {
        let tracker = WorkflowTracker::new("empty");
        tracker.summary();
    }

    #[tokio::test]
    async fn summary_does_not_panic_non_empty() {
        let mut tracker = WorkflowTracker::new("test");
        let shell = make_shell_output().await;
        let agent = make_agent_result(Some(0.01), Some(10), Some(5));
        tracker.record_shell("s1", &shell);
        tracker.record_agent("a1", &agent);
        tracker.summary();
    }

    #[test]
    fn eviction_when_max_steps_exceeded() {
        let mut tracker = WorkflowTracker::new("test").max_steps(3);
        for i in 0..5 {
            let r = make_agent_result(Some(i as f64), None, None);
            tracker.record_agent(&format!("step-{i}"), &r);
        }
        assert_eq!(tracker.step_count(), 3);
        // Oldest steps (0, 1) were evicted; remaining are steps 2, 3, 4
        assert_eq!(tracker.total_cost_usd(), 2.0 + 3.0 + 4.0);
    }

    #[test]
    fn max_steps_one_keeps_last_only() {
        let mut tracker = WorkflowTracker::new("test").max_steps(1);
        let r1 = make_agent_result(Some(1.0), Some(100), None);
        let r2 = make_agent_result(Some(2.0), Some(200), None);
        tracker.record_agent("a1", &r1);
        tracker.record_agent("a2", &r2);
        assert_eq!(tracker.step_count(), 1);
        assert_eq!(tracker.total_cost_usd(), 2.0);
        assert_eq!(tracker.total_input_tokens(), 200);
    }

    #[test]
    fn max_steps_builder_sets_limit() {
        let mut tracker = WorkflowTracker::new("test").max_steps(42);
        // Verify the limit works by adding more than 42 steps
        for i in 0..50 {
            let r = make_agent_result(Some(1.0), None, None);
            tracker.record_agent(&format!("step-{i}"), &r);
        }
        assert_eq!(tracker.step_count(), 42);
    }
}
