//! [`WorkflowContext`] — execution context for dynamic workflows.
//!
//! Provides step execution methods that automatically persist results to the
//! store. Each call to [`shell`](WorkflowContext::shell),
//! [`http`](WorkflowContext::http), [`agent`](WorkflowContext::agent), or
//! [`workflow`](WorkflowContext::workflow) creates a step record, executes the
//! operation, captures the output, and returns a
//! [`StepOutput`](crate::executor::StepOutput) that the next step can
//! reference.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_engine::context::WorkflowContext;
//! use ironflow_engine::config::{ShellConfig, AgentStepConfig};
//! use ironflow_engine::error::EngineError;
//!
//! # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
//! let build = ctx.shell("build", ShellConfig::new("cargo build")).await?;
//! let review = ctx.agent("review", AgentStepConfig::new(
//!     &format!("Build output:\n{}", build.stdout())
//! )).await?;
//! # Ok(())
//! # }
//! ```

mod accessors;
mod artifacts;
mod error_handlers;
mod failure;
mod guard;
mod lifecycle;
mod steps;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;

use ironflow_core::decision::DecisionProvider;
use ironflow_core::provider::AgentProvider;
use ironflow_core::trace_context::WorkflowTraceContext;
use ironflow_store::models::Step;
use ironflow_store::store::Store;

use crate::artifact::ArtifactSink;
use crate::config::StepConfig;
use crate::executor::{StepInterceptor, StepResult};
use crate::guard::{SharedGuardState, WorkflowGuardConfig};
use crate::handler::WorkflowHandler;
use crate::log_sender::LogSender;
use crate::notify::WorkflowEventBus;
use crate::operation::OperationContext;
use crate::plan::SharedPlanRecorder;

/// Label set on every child run of a sub-workflow step, holding the id of the
/// run that started it.
///
/// The root of the chain is recorded under
/// [`LABEL_ROOT_RUN_ID`](ironflow_core::provider::LABEL_ROOT_RUN_ID). Both are
/// set on the child run when it is created, so a suspended child can be
/// listed by label and resumed through its root.
///
/// # Examples
///
/// ```no_run
/// use std::collections::HashMap;
/// use ironflow_engine::context::PARENT_RUN_ID_LABEL;
/// use ironflow_store::models::RunFilter;
/// use uuid::Uuid;
///
/// # fn example(parent: Uuid) {
/// let children = RunFilter {
///     labels: Some(HashMap::from([(PARENT_RUN_ID_LABEL.to_string(), parent.to_string())])),
///     ..RunFilter::default()
/// };
/// # }
/// ```
pub const PARENT_RUN_ID_LABEL: &str = "ironflow.io/parent-run-id";

/// Callback type for resolving workflow handlers by name.
pub(crate) type HandlerResolver =
    Arc<dyn Fn(&str) -> Option<Arc<dyn WorkflowHandler>> + Send + Sync>;

/// Execution context for a single workflow run.
///
/// Tracks the current step position and provides convenience methods
/// for executing operations with automatic persistence.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::context::WorkflowContext;
/// use ironflow_engine::config::ShellConfig;
/// use ironflow_engine::error::EngineError;
///
/// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
/// let result = ctx.shell("greet", ShellConfig::new("echo hello")).await?;
/// assert!(result.stdout().contains("hello"));
/// # Ok(())
/// # }
/// ```
pub struct WorkflowContext {
    run_id: Uuid,
    /// The top-level run: `run_id` itself, or the parent's root for a
    /// sub-workflow. Stamped on agent pods so a retry releases the children.
    root_run_id: Uuid,
    workflow_name: String,
    store: Arc<dyn Store>,
    provider: Arc<dyn AgentProvider>,
    /// Optional decision backend (System One / Jev) for `ctx.decision(...)`.
    /// `None` when no decision provider was wired: a decision step then fails
    /// explicitly instead of silently doing nothing.
    decision_provider: Option<Arc<dyn DecisionProvider>>,
    handler_resolver: Option<HandlerResolver>,
    position: u32,
    /// IDs of the last executed step(s) -- used to record DAG dependencies.
    last_step_ids: Vec<Uuid>,
    /// Accumulated cost across all steps in this run.
    total_cost_usd: Decimal,
    /// Accumulated duration across all steps.
    total_duration_ms: u64,
    /// Cumulative cost cap for this run, resolved at creation. `None` = no cap.
    max_cost_usd: Option<Decimal>,
    /// Cost already spent by ancestor runs when this context belongs to a
    /// sub-workflow. Zero for a top-level run.
    inherited_cost_usd: Decimal,
    /// Steps from a previous execution of the *same* attempt, keyed by position.
    /// Used when resuming after approval to replay completed steps.
    replay_steps: HashMap<u32, Step>,
    /// All steps of a previous execution of the *same* attempt, keyed by
    /// `(position, step name)`. A `parallel` wave shares one position across
    /// several steps, which `replay_steps` cannot represent -- this index lets
    /// `parallel()` check that every step of a wave already completed before
    /// replaying the whole wave from the store, without re-running any item.
    replay_wave_steps: HashMap<(u32, String), Step>,
    /// Approvals granted in an *earlier* attempt, keyed by position, holding the
    /// attempt that granted them. An approval is carried by the run, not by the
    /// attempt, so a retry never asks a human to approve the same gate twice.
    granted_approvals: HashMap<u32, u32>,
    /// Human inputs answered in an *earlier* attempt, keyed by position:
    /// (attempt, answer). Like an approval, an answer is carried by the run, so
    /// a retry never asks a human to answer the same input twice.
    answered_inputs: HashMap<u32, (u32, Value)>,
    /// Which run attempt this context is executing (1-based).
    attempt: u32,
    /// Wall-clock duration already recorded on the run by previous attempts.
    /// Added to this attempt's duration when the run is finalized.
    carried_duration_ms: u64,
    /// Optional sender for real-time log streaming.
    log_sender: Option<LogSender>,
    /// Where artifact bytes are read and written. `None` when no artifact
    /// storage is configured: steps that declare artifacts then fail explicitly
    /// instead of silently dropping their files.
    artifact_sink: Option<Arc<dyn ArtifactSink>>,
    /// Set to `true` when at least one `allow_failure` step failed.
    has_allowed_failure: bool,
    /// Error handlers registered via [`on_error`](Self::on_error).
    error_handlers: Vec<OnErrorHandler>,
    /// Shared guard state for workflow execution limits.
    guard_state: Option<SharedGuardState>,
    /// Guard configuration for this workflow run.
    guard_config: Option<WorkflowGuardConfig>,
    /// Accumulated step results for post-execution inspection.
    step_results: Vec<StepResult>,
    /// Optional event bus for per-run real-time monitoring.
    event_bus: Option<WorkflowEventBus>,
    /// Optional hook that resolves steps without executing them. `None` in
    /// production; set by [`crate::testing::TestEngine`].
    interceptor: Option<Arc<dyn StepInterceptor>>,
    /// W3C trace context for distributed tracing propagation.
    trace_context: WorkflowTraceContext,
    /// Shared operation context for custom operations.
    operation_ctx: Option<OperationContext>,
    /// When the run was created, set by the engine. Bounds the signals a wait
    /// step accepts: a signal received before the run existed is not for it.
    /// `None` falls back to reading the run from the store.
    run_created_at: Option<DateTime<Utc>>,
    /// Set when the context is recording an execution plan instead of running.
    /// Every step method checks this first and records intent without executing.
    plan: Option<SharedPlanRecorder>,
}

/// A registered error handler that fires when a subsequent step fails.
struct OnErrorHandler {
    name: String,
    config: StepConfig,
}

impl fmt::Debug for WorkflowContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkflowContext")
            .field("run_id", &self.run_id)
            .field("position", &self.position)
            .field("total_cost_usd", &self.total_cost_usd)
            .field("inherited_cost_usd", &self.inherited_cost_usd)
            .field("max_cost_usd", &self.max_cost_usd)
            .field("planning", &self.plan.is_some())
            .finish_non_exhaustive()
    }
}
