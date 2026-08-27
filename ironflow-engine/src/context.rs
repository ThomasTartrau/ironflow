//! [`WorkflowContext`] — execution context for dynamic workflows.
//!
//! Provides step execution methods that automatically persist results to the
//! store. Each call to [`shell`](WorkflowContext::shell),
//! [`http`](WorkflowContext::http), [`agent`](WorkflowContext::agent), or
//! [`workflow`](WorkflowContext::workflow) creates a step record, executes the
//! operation, captures the output, and returns a [`StepOutput`] that the next
//! step can reference.
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
//!     &format!("Build output:\n{}", build.output["stdout"])
//! )).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::task::{Id, JoinSet};
use tracing::{Span, error, info, warn};
use uuid::Uuid;

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::provider::AgentProvider;
use ironflow_store::models::{
    ArtifactLookup, NewRun, NewStep, NewStepDependency, RunStatus, RunUpdate, Step, StepKind,
    StepStatus, StepUpdate, TriggerKind,
};
use ironflow_store::store::Store;

use ironflow_artifacts::name::guess_content_type;
use ironflow_artifacts::stream_from_bytes;
use ironflow_store::entities::Artifact;

use crate::artifact::{
    ArtifactSink, ArtifactUpload, StepLocation, collect_outputs, materialize_inputs,
};
use crate::budget::step_budget_usd;
use crate::config::{
    AgentStepConfig, ApprovalConfig, HttpConfig, ShellConfig, StepConfig, WorkflowStepConfig,
};
use crate::error::EngineError;
use crate::executor::{ParallelStepResult, StepOutput, execute_step_config};
use crate::guard::{SharedGuardState, WorkflowGuardConfig, WorkflowRejection};
use crate::handler::WorkflowHandler;
use crate::log_sender::{LogSender, StepLogSender};
use crate::operation::Operation;

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
/// assert!(result.output["stdout"].as_str().unwrap().contains("hello"));
/// # Ok(())
/// # }
/// ```
pub struct WorkflowContext {
    run_id: Uuid,
    store: Arc<dyn Store>,
    provider: Arc<dyn AgentProvider>,
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
    /// Approvals granted in an *earlier* attempt, keyed by position, holding the
    /// attempt that granted them. An approval is carried by the run, not by the
    /// attempt, so a retry never asks a human to approve the same gate twice.
    granted_approvals: HashMap<u32, u32>,
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
    /// Optional event bus for per-run real-time monitoring.
    event_bus: Option<crate::notify::WorkflowEventBus>,
}

/// A registered error handler that fires when a subsequent step fails.
struct OnErrorHandler {
    name: String,
    config: StepConfig,
}

impl WorkflowContext {
    /// Create a new context for a run.
    ///
    /// Not typically called directly — the [`Engine`](crate::engine::Engine)
    /// creates this when executing a [`WorkflowHandler`].
    pub fn new(run_id: Uuid, store: Arc<dyn Store>, provider: Arc<dyn AgentProvider>) -> Self {
        Self {
            run_id,
            store,
            provider,
            handler_resolver: None,
            position: 0,
            last_step_ids: Vec::new(),
            total_cost_usd: Decimal::ZERO,
            total_duration_ms: 0,
            max_cost_usd: None,
            inherited_cost_usd: Decimal::ZERO,
            replay_steps: HashMap::new(),
            granted_approvals: HashMap::new(),
            attempt: 1,
            carried_duration_ms: 0,
            log_sender: None,
            artifact_sink: None,
            has_allowed_failure: false,
            error_handlers: Vec::new(),
            guard_state: None,
            guard_config: None,
            event_bus: None,
        }
    }

    /// Create a new context with a handler resolver for sub-workflow support.
    ///
    /// The resolver is called when [`workflow`](Self::workflow) is invoked to
    /// look up registered handlers by name.
    pub(crate) fn with_handler_resolver(
        run_id: Uuid,
        store: Arc<dyn Store>,
        provider: Arc<dyn AgentProvider>,
        resolver: HandlerResolver,
    ) -> Self {
        Self {
            run_id,
            store,
            provider,
            handler_resolver: Some(resolver),
            position: 0,
            last_step_ids: Vec::new(),
            total_cost_usd: Decimal::ZERO,
            total_duration_ms: 0,
            max_cost_usd: None,
            inherited_cost_usd: Decimal::ZERO,
            replay_steps: HashMap::new(),
            granted_approvals: HashMap::new(),
            attempt: 1,
            carried_duration_ms: 0,
            log_sender: None,
            artifact_sink: None,
            has_allowed_failure: false,
            error_handlers: Vec::new(),
            guard_state: None,
            guard_config: None,
            event_bus: None,
        }
    }

    /// Attach a log sender for real-time step output streaming.
    pub fn set_log_sender(&mut self, sender: LogSender) {
        self.log_sender = Some(sender);
    }

    /// Attach the backend that stores and serves artifact bytes.
    ///
    /// Without one, any step that declares an output or calls
    /// [`put_artifact`](Self::put_artifact) fails with
    /// [`EngineError::ArtifactsUnavailable`]. Every other step is unaffected,
    /// so an existing deployment keeps working until artifacts are configured.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use ironflow_engine::artifact::ArtifactSink;
    /// use ironflow_engine::context::WorkflowContext;
    ///
    /// # fn example(ctx: &mut WorkflowContext, sink: Arc<dyn ArtifactSink>) {
    /// ctx.set_artifact_sink(sink);
    /// # }
    /// ```
    pub fn set_artifact_sink(&mut self, sink: Arc<dyn ArtifactSink>) {
        self.artifact_sink = Some(sink);
    }

    /// Attach a workflow guard configuration and shared state.
    ///
    /// When set, the guard is checked before every sub-workflow invocation.
    /// The shared state is propagated to child workflows so that limits
    /// apply globally across the entire run tree.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::guard::{WorkflowGuardConfig, new_shared_guard_state};
    ///
    /// # fn example(ctx: &mut WorkflowContext) {
    /// ctx.set_guard(WorkflowGuardConfig::default(), new_shared_guard_state());
    /// # }
    /// ```
    pub fn set_guard(&mut self, config: WorkflowGuardConfig, state: SharedGuardState) {
        self.guard_config = Some(config);
        self.guard_state = Some(state);
    }

    /// The current guard configuration, if any.
    pub fn guard_config(&self) -> Option<&WorkflowGuardConfig> {
        self.guard_config.as_ref()
    }

    /// Attach a [`WorkflowEventBus`](crate::notify::WorkflowEventBus) for
    /// per-run real-time monitoring.
    ///
    /// When set, step transitions automatically publish
    /// [`WorkflowEvent`](crate::notify::WorkflowEvent)s to the bus.
    pub fn set_event_bus(&mut self, bus: crate::notify::WorkflowEventBus) {
        self.event_bus = Some(bus);
    }

    /// The artifact backend, or an explicit error when none is configured.
    fn artifact_sink(&self) -> Result<&Arc<dyn ArtifactSink>, EngineError> {
        self.artifact_sink.as_ref().ok_or_else(|| {
            EngineError::ArtifactsUnavailable(
                "no artifact storage is attached to this run".to_string(),
            )
        })
    }

    /// Store an in-memory payload as an artifact of the given step.
    ///
    /// The declarative [`ShellConfig::output`](crate::config::ShellConfig::output)
    /// covers shell steps; this covers custom operations and agent steps, which
    /// have no working directory to collect from.
    ///
    /// The MIME type is guessed from `name` unless `content_type` is set.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ArtifactsUnavailable`] when no backend is
    /// attached, [`EngineError::Artifact`] when the name is invalid or storage
    /// fails, and [`EngineError::Store`] when the step already owns that name.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use uuid::Uuid;
    ///
    /// # async fn example(ctx: &WorkflowContext, step_id: Uuid) -> Result<(), EngineError> {
    /// let artifact = ctx
    ///     .put_artifact(step_id, "summary.json", None, br#"{"ok":true}"#.to_vec())
    ///     .await?;
    /// assert_eq!(artifact.content_type, "application/json");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn put_artifact(
        &self,
        step_id: Uuid,
        name: &str,
        content_type: Option<&str>,
        content: Vec<u8>,
    ) -> Result<Artifact, EngineError> {
        let sink = self.artifact_sink()?;
        sink.put(
            ArtifactUpload {
                run_id: self.run_id,
                step_id,
                name: name.to_string(),
                content_type: content_type
                    .map(str::to_string)
                    .unwrap_or_else(|| guess_content_type(name)),
            },
            stream_from_bytes(content),
        )
        .await
    }

    /// Read back an artifact produced earlier in this run.
    ///
    /// Resolution follows the same rule as a declared input: same run and
    /// attempt, steps positioned strictly before the current one, closest
    /// producer wins.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ArtifactNotFound`] when nothing matches,
    /// [`EngineError::ArtifactsUnavailable`] when no backend is attached, and
    /// [`EngineError::Artifact`] when the bytes cannot be read.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &WorkflowContext) -> Result<(), EngineError> {
    /// let bytes = ctx.get_artifact("build", "report.html").await?;
    /// println!("{} bytes", bytes.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_artifact(&self, step: &str, name: &str) -> Result<Vec<u8>, EngineError> {
        let sink = self.artifact_sink()?;

        let artifact = self
            .store
            .find_artifact_for_input(ArtifactLookup {
                run_id: self.run_id,
                attempt: self.attempt,
                before_position: self.position,
                step_name: step.to_string(),
                name: name.to_string(),
            })
            .await?
            .ok_or_else(|| EngineError::ArtifactNotFound {
                step: step.to_string(),
                name: name.to_string(),
            })?;

        let mut content = sink.get(&artifact).await?;
        let mut buffer = Vec::with_capacity(artifact.size_bytes as usize);
        while let Some(chunk) = content.next().await {
            let chunk = chunk?;
            buffer.extend_from_slice(chunk.as_ref());
        }

        Ok(buffer)
    }

    /// Place a shell step's declared inputs in its working directory.
    ///
    /// A step that declares none needs no backend, so the check for one only
    /// happens when there is something to materialize.
    async fn prepare_step_inputs(
        &self,
        config: &StepConfig,
        position: u32,
    ) -> Result<(), EngineError> {
        let StepConfig::Shell(shell) = config else {
            return Ok(());
        };
        if shell.inputs.is_empty() {
            return Ok(());
        }

        materialize_inputs(
            self.artifact_sink()?,
            &self.store,
            shell,
            StepLocation {
                run_id: self.run_id,
                attempt: self.attempt,
                position,
            },
        )
        .await
    }

    /// Store a shell step's declared outputs.
    ///
    /// On a failed step this is best-effort: the collection error is logged and
    /// swallowed so it never masks the failure that actually stopped the step.
    async fn store_step_outputs(
        &self,
        config: &StepConfig,
        step_id: Uuid,
        step_name: &str,
        step_succeeded: bool,
    ) -> Result<(), EngineError> {
        let StepConfig::Shell(shell) = config else {
            return Ok(());
        };
        if shell.outputs.is_empty() {
            return Ok(());
        }

        let sink = match self.artifact_sink() {
            Ok(sink) => sink,
            Err(err) if step_succeeded => return Err(err),
            Err(err) => {
                warn!(
                    run_id = %self.run_id,
                    step = %step_name,
                    error = %err,
                    "cannot collect outputs of a failed step"
                );
                return Ok(());
            }
        };

        let collected =
            collect_outputs(sink, shell, self.run_id, step_id, step_name, step_succeeded).await;

        match collected {
            Ok(()) => Ok(()),
            Err(err) if step_succeeded => Err(err),
            Err(err) => {
                warn!(
                    run_id = %self.run_id,
                    step = %step_name,
                    error = %err,
                    "failed to collect outputs of a failed step"
                );
                Ok(())
            }
        }
    }

    /// Seed the context with the run's attempt number and the totals already
    /// accumulated by previous attempts.
    ///
    /// Called by the engine before executing a handler. Steps created by this
    /// context belong to `attempt`, and the cost and duration it reports at the
    /// end cover the whole run, not just this attempt.
    pub(crate) fn carry_over_run_totals(
        &mut self,
        attempt: u32,
        cost_usd: Decimal,
        duration_ms: u64,
    ) {
        self.attempt = attempt;
        self.total_cost_usd = cost_usd;
        self.carried_duration_ms = duration_ms;
    }

    /// Wall-clock duration already recorded on the run by previous attempts.
    pub(crate) fn carried_duration_ms(&self) -> u64 {
        self.carried_duration_ms
    }

    /// The run attempt this context is executing (1-based).
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Set the cumulative cost cap enforced before every agent step.
    ///
    /// Called by the [`Engine`](crate::engine::Engine) with the run's persisted
    /// `max_cost_usd`. `None` disables the check.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use rust_decimal::Decimal;
    ///
    /// # fn example(ctx: &mut WorkflowContext) {
    /// ctx.set_max_cost_usd(Some(Decimal::new(200, 2))); // $2.00
    /// # }
    /// ```
    pub fn set_max_cost_usd(&mut self, cap: Option<Decimal>) {
        self.max_cost_usd = cap;
    }

    /// The cumulative cost cap of this run, if any.
    pub fn max_cost_usd(&self) -> Option<Decimal> {
        self.max_cost_usd
    }

    /// Total cost charged against the cap: this run plus every ancestor run.
    ///
    /// For a top-level run this equals [`total_cost_usd`](Self::total_cost_usd).
    /// For a sub-workflow it also includes what the parent chain already spent.
    pub fn charged_cost_usd(&self) -> Decimal {
        self.inherited_cost_usd + self.total_cost_usd
    }

    /// Reject the upcoming agent work when it would cross the run's cost cap.
    ///
    /// `step_budget` is the declared budget of the step (or the sum of budgets
    /// for a parallel wave). Called *before* any step record is created so a
    /// refused run never launches the work it could not afford.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::RunBudgetExceeded`] when
    /// `charged_cost + step_budget` exceeds the cap.
    fn check_run_budget(&self, step_budget: Decimal) -> Result<(), EngineError> {
        let Some(limit) = self.max_cost_usd else {
            return Ok(());
        };

        let spent = self.charged_cost_usd();
        if spent + step_budget <= limit {
            return Ok(());
        }

        error!(
            run_id = %self.run_id,
            limit_usd = %limit,
            spent_usd = %spent,
            step_budget_usd = %step_budget,
            "run cost cap reached, refusing agent step"
        );

        Err(EngineError::RunBudgetExceeded {
            run_id: self.run_id,
            limit_usd: limit,
            spent_usd: spent,
            step_budget_usd: step_budget,
        })
    }

    /// Load existing steps from the store for replay after approval.
    ///
    /// Called by the engine when resuming a run. All completed steps
    /// and the approved approval step are indexed by position so that
    /// `execute_step` and `approval` can skip them.
    ///
    /// Only steps of the current attempt are replayed: positions repeat across
    /// attempts, so replaying an earlier attempt's steps would skip the whole
    /// workflow. The one exception is an approval already granted in an earlier
    /// attempt -- approval is carried by the run, not by the attempt, so a human
    /// is never asked to approve the same gate twice.
    pub(crate) async fn load_replay_steps(&mut self) -> Result<(), EngineError> {
        let steps = self.store.list_steps(self.run_id).await?;
        for step in steps {
            let dominated = matches!(
                step.status.state,
                StepStatus::Completed | StepStatus::Running | StepStatus::AwaitingApproval
            );
            if !dominated {
                continue;
            }

            if step.attempt == self.attempt {
                self.replay_steps.insert(step.position, step);
            } else if step.kind == StepKind::Approval && step.status.state == StepStatus::Completed
            {
                self.granted_approvals.insert(step.position, step.attempt);
            }
        }
        Ok(())
    }

    /// The run ID this context is executing for.
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    /// Accumulated cost across all executed steps so far.
    pub fn total_cost_usd(&self) -> Decimal {
        self.total_cost_usd
    }

    /// Whether at least one `allow_failure` step failed during this run.
    pub fn has_allowed_failure(&self) -> bool {
        self.has_allowed_failure
    }

    /// Accumulated duration across all executed steps so far.
    pub fn total_duration_ms(&self) -> u64 {
        self.total_duration_ms
    }

    /// Execute multiple steps concurrently (wait-all model).
    ///
    /// All steps in the batch execute in parallel via `tokio::JoinSet`.
    /// Each step is recorded with the same `position` (execution wave).
    /// Dependencies on previous steps are recorded automatically.
    ///
    /// When `fail_fast` is true, remaining steps are aborted on the first
    /// failure. When false, all steps run to completion and the first
    /// error is returned.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if any step fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::{StepConfig, ShellConfig};
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let results = ctx.parallel(
    ///     vec![
    ///         ("test-unit", StepConfig::Shell(ShellConfig::new("cargo test --lib"))),
    ///         ("lint", StepConfig::Shell(ShellConfig::new("cargo clippy"))),
    ///     ],
    ///     true,
    /// ).await?;
    ///
    /// for r in &results {
    ///     println!("{}: {:?}", r.name, r.output.output);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn parallel(
        &mut self,
        steps: Vec<(&str, StepConfig)>,
        fail_fast: bool,
    ) -> Result<Vec<ParallelStepResult>, EngineError> {
        if steps.is_empty() {
            return Ok(Vec::new());
        }

        // Guard timeout: checked before launching the wave.
        self.check_guard_timeout()?;

        // Cost cap: the whole wave is charged at once. Refused before any step
        // record is created, so nothing in the wave starts.
        let wave_budget: Decimal = steps
            .iter()
            .filter_map(|(_, config)| match config {
                StepConfig::Agent(agent_config) => Some(agent_config.max_budget_usd),
                _ => None,
            })
            .map(step_budget_usd)
            .sum();
        self.check_run_budget(wave_budget)?;

        let wave_position = self.position;
        self.position += 1;

        let now = Utc::now();
        let mut step_records: Vec<(Uuid, String, StepConfig)> = Vec::with_capacity(steps.len());

        for (name, config) in &steps {
            let kind = config.kind();
            let step = self
                .store
                .create_step(NewStep {
                    run_id: self.run_id,
                    name: name.to_string(),
                    kind,
                    position: wave_position,
                    input: Some(serde_json::to_value(config)?),
                    is_error_handler: false,
                })
                .await?;

            self.start_step(step.id, now).await?;

            // Inputs are materialized before any step in the wave starts, so a
            // missing one fails the wave rather than a half-run command.
            if let Err(err) = self.prepare_step_inputs(config, wave_position).await {
                self.fail_step(step.id, &err).await;
                if !config.allow_failure() {
                    return Err(err);
                }
                self.has_allowed_failure = true;
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    error = %err,
                    "parallel step input preparation failed but allow_failure is set, skipping"
                );
                continue;
            }

            step_records.push((step.id, name.to_string(), config.clone()));
        }

        let mut join_set = JoinSet::new();
        let mut task_index: HashMap<Id, usize> = HashMap::new();
        let parallel_timeout = self.guard_remaining_timeout();
        for (idx, (step_id, step_name, config)) in step_records.iter().enumerate() {
            let provider = self.provider.clone();
            let config = config.clone();
            let step_log_sender = self
                .log_sender
                .as_ref()
                .map(|s| StepLogSender::new(s.clone(), self.run_id, *step_id, step_name.clone()));
            let handle = join_set.spawn(async move {
                let result = match parallel_timeout {
                    Some(dur) => {
                        match tokio::time::timeout(
                            dur,
                            execute_step_config(&config, &provider, step_log_sender),
                        )
                        .await
                        {
                            Ok(r) => r,
                            Err(_elapsed) => {
                                Err(EngineError::from(WorkflowRejection::WorkflowTimeout {
                                    elapsed_secs: 0,
                                    max: 0,
                                }))
                            }
                        }
                    }
                    None => execute_step_config(&config, &provider, step_log_sender).await,
                };
                (idx, result)
            });
            task_index.insert(handle.id(), idx);
        }

        // JoinSet returns in completion order; indexed_results restores input order.
        let mut indexed_results: Vec<Option<Result<StepOutput, String>>> =
            vec![None; step_records.len()];
        let mut first_error: Option<EngineError> = None;

        while let Some(join_result) = join_set.join_next().await {
            let (idx, step_result) = match join_result {
                Ok(r) => r,
                Err(e) => {
                    let error_msg = format!("join error: {e}");
                    if let Some(&idx) = task_index.get(&e.id()) {
                        let (step_id, step_name, _) = &step_records[idx];
                        let completed_at = Utc::now();
                        error!(
                            run_id = %self.run_id,
                            step = %step_name,
                            error = %error_msg,
                            "parallel step panicked or was cancelled"
                        );
                        if let Err(store_err) = self
                            .store
                            .update_step(
                                *step_id,
                                StepUpdate {
                                    status: Some(StepStatus::Failed),
                                    error: Some(error_msg.clone()),
                                    completed_at: Some(completed_at),
                                    ..StepUpdate::default()
                                },
                            )
                            .await
                        {
                            error!(
                                run_id = %self.run_id,
                                step_id = %step_id,
                                error = %store_err,
                                "failed to persist JoinError for step"
                            );
                        }
                        indexed_results[idx] = Some(Err(error_msg.clone()));
                    }
                    if first_error.is_none() {
                        first_error = Some(EngineError::StepConfig(error_msg));
                    }
                    if fail_fast {
                        join_set.abort_all();
                    }
                    continue;
                }
            };

            let (step_id, step_name, step_config) = &step_records[idx];
            let completed_at = Utc::now();

            if let Err(err) = self
                .store_step_outputs(step_config, *step_id, step_name, step_result.is_ok())
                .await
            {
                self.fail_step(*step_id, &err).await;
                indexed_results[idx] = Some(Err(err.to_string()));
                if first_error.is_none() {
                    first_error = Some(err);
                }
                if fail_fast {
                    join_set.abort_all();
                }
                continue;
            }

            match step_result {
                Ok(output) => {
                    self.total_cost_usd += output.cost_usd;
                    self.total_duration_ms += output.duration_ms;

                    // Record token usage in the guard for agent steps.
                    if matches!(step_config, StepConfig::Agent(_)) {
                        let tokens = output
                            .input_tokens
                            .unwrap_or(0)
                            .saturating_add(output.output_tokens.unwrap_or(0));
                        if tokens > 0
                            && let Err(guard_err) = self.guard_record_tokens(tokens)
                        {
                            if first_error.is_none() {
                                first_error = Some(guard_err);
                            }
                            if fail_fast {
                                join_set.abort_all();
                            }
                        }
                    }

                    let debug_messages_json = output.debug_messages_json();

                    self.store
                        .update_step(
                            *step_id,
                            StepUpdate {
                                status: Some(StepStatus::Completed),
                                output: Some(output.output.clone()),
                                duration_ms: Some(output.duration_ms),
                                cost_usd: Some(output.cost_usd),
                                input_tokens: output.input_tokens,
                                output_tokens: output.output_tokens,
                                completed_at: Some(completed_at),
                                debug_messages: debug_messages_json,
                                ..StepUpdate::default()
                            },
                        )
                        .await?;

                    info!(
                        run_id = %self.run_id,
                        step = %step_name,
                        duration_ms = output.duration_ms,
                        "parallel step completed"
                    );

                    indexed_results[idx] = Some(Ok(output));
                }
                Err(err) => {
                    let err_msg = err.to_string();
                    let debug_messages_json = extract_debug_messages_from_error(&err);
                    let partial = extract_partial_usage_from_error(&err);
                    let raw_response_output = extract_raw_response_from_error(&err);

                    if let Some(ref usage) = partial {
                        if let Some(cost) = usage.cost_usd {
                            self.total_cost_usd += cost;
                        }
                        if let Some(dur) = usage.duration_ms {
                            self.total_duration_ms += dur;
                        }
                    }

                    if let Err(store_err) = self
                        .store
                        .update_step(
                            *step_id,
                            StepUpdate {
                                status: Some(StepStatus::Failed),
                                error: Some(err_msg.clone()),
                                output: raw_response_output.clone(),
                                completed_at: Some(completed_at),
                                debug_messages: debug_messages_json,
                                duration_ms: partial.as_ref().and_then(|p| p.duration_ms),
                                cost_usd: partial.as_ref().and_then(|p| p.cost_usd),
                                input_tokens: partial.as_ref().and_then(|p| p.input_tokens),
                                output_tokens: partial.as_ref().and_then(|p| p.output_tokens),
                                ..StepUpdate::default()
                            },
                        )
                        .await
                    {
                        tracing::error!(
                            step_id = %step_id,
                            error = %store_err,
                            "failed to persist parallel step failure"
                        );
                    }

                    if step_config.allow_failure() {
                        self.has_allowed_failure = true;
                        info!(
                            run_id = %self.run_id,
                            step = %step_name,
                            error = %err_msg,
                            "parallel step failed but allow_failure is set, continuing"
                        );
                        indexed_results[idx] = Some(Ok(allowed_failure_output(
                            &err_msg,
                            raw_response_output,
                            partial.as_ref(),
                        )));
                    } else {
                        indexed_results[idx] = Some(Err(err_msg.clone()));

                        if first_error.is_none() {
                            first_error = Some(err);
                        }

                        if fail_fast {
                            join_set.abort_all();
                        }
                    }
                }
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }

        self.last_step_ids = step_records.iter().map(|(id, _, _)| *id).collect();

        // Build results in original order.
        let results: Vec<ParallelStepResult> = step_records
            .iter()
            .enumerate()
            .map(|(idx, (step_id, name, _))| {
                let output = match indexed_results[idx].take() {
                    Some(Ok(o)) => o,
                    _ => unreachable!("all steps succeeded if no error returned"),
                };
                ParallelStepResult {
                    name: name.clone(),
                    output,
                    step_id: *step_id,
                }
            })
            .collect();

        Ok(results)
    }

    /// Execute a shell step.
    ///
    /// Creates the step record, runs the command, persists the result,
    /// and returns the output for use in subsequent steps.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the command fails or the store errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let files = ctx.shell("list", ShellConfig::new("ls -la")).await?;
    /// println!("stdout: {}", files.output["stdout"]);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shell(
        &mut self,
        name: &str,
        config: ShellConfig,
    ) -> Result<StepOutput, EngineError> {
        self.execute_step(name, StepKind::Shell, StepConfig::Shell(config))
            .await
    }

    /// Execute an HTTP step.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the request fails or the store errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::HttpConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let resp = ctx.http("health", HttpConfig::get("https://api.example.com/health")).await?;
    /// println!("status: {}", resp.output["status"]);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn http(
        &mut self,
        name: &str,
        config: HttpConfig,
    ) -> Result<StepOutput, EngineError> {
        self.execute_step(name, StepKind::Http, StepConfig::Http(config))
            .await
    }

    /// Execute an agent step.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the agent invocation fails or the store errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::AgentStepConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let review = ctx.agent("review", AgentStepConfig::new("Review the code")).await?;
    /// println!("review: {}", review.output);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn agent(
        &mut self,
        name: &str,
        config: impl Into<AgentStepConfig>,
    ) -> Result<StepOutput, EngineError> {
        self.execute_step(name, StepKind::Agent, StepConfig::Agent(config.into()))
            .await
    }

    /// Create a human approval gate.
    ///
    /// On first execution, records an approval step and returns
    /// [`EngineError::ApprovalRequired`] to suspend the run. The engine
    /// transitions the run to `AwaitingApproval`.
    ///
    /// On resume (after a human approved via the API), the approval step
    /// is replayed: it is marked as `Completed` and execution continues
    /// past it. Multiple approval gates in the same handler work -- each
    /// one pauses and resumes independently.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ApprovalRequired`] to pause the run on
    /// first execution. Returns other [`EngineError`] variants on store
    /// failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ApprovalConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// ctx.approval("deploy-gate", ApprovalConfig::new("Approve deployment?")).await?;
    /// // Execution continues here after approval
    /// # Ok(())
    /// # }
    /// ```
    pub async fn approval(
        &mut self,
        name: &str,
        config: ApprovalConfig,
    ) -> Result<(), EngineError> {
        let position = self.position;
        self.position += 1;

        // Replay: if this approval step exists from a prior execution,
        // the run was approved -- mark it completed (if not already) and continue.
        if let Some(existing) = self.replay_steps.get(&position)
            && existing.kind == StepKind::Approval
        {
            if existing.status.state == StepStatus::AwaitingApproval {
                self.store
                    .update_step(
                        existing.id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            completed_at: Some(Utc::now()),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;
            }

            self.last_step_ids = vec![existing.id];
            info!(
                run_id = %self.run_id,
                step = %name,
                position,
                "approval step replayed (approved)"
            );
            return Ok(());
        }

        // Carried over: a human already approved this gate in an earlier
        // attempt. Record a fresh step in the current attempt so that each
        // attempt keeps a complete, self-contained DAG, and continue.
        if let Some(&granted_in) = self.granted_approvals.get(&position) {
            let step = self
                .store
                .create_step(NewStep {
                    run_id: self.run_id,
                    name: name.to_string(),
                    kind: StepKind::Approval,
                    position,
                    input: Some(serde_json::to_value(&config)?),
                    is_error_handler: false,
                })
                .await?;

            let now = Utc::now();
            self.start_step(step.id, now).await?;
            self.store
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Completed),
                        output: Some(json!({"approved_in_attempt": granted_in})),
                        completed_at: Some(now),
                        ..StepUpdate::default()
                    },
                )
                .await?;

            self.last_step_ids = vec![step.id];
            info!(
                run_id = %self.run_id,
                step = %name,
                position,
                granted_in_attempt = granted_in,
                attempt = self.attempt,
                "approval carried over from a previous attempt"
            );
            return Ok(());
        }

        // First execution: create the approval step and suspend.
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                name: name.to_string(),
                kind: StepKind::Approval,
                position,
                input: Some(serde_json::to_value(&config)?),
                is_error_handler: false,
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        // Transition the step to AwaitingApproval so it reflects
        // the suspended state on the dashboard.
        self.store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    ..StepUpdate::default()
                },
            )
            .await?;

        self.last_step_ids = vec![step.id];

        Err(EngineError::ApprovalRequired {
            run_id: self.run_id,
            step_id: step.id,
            message: config.message().to_string(),
        })
    }

    /// Record a step as explicitly skipped.
    ///
    /// Use this inside an `if`/`else` branch when a step should not execute
    /// but must still appear in the DAG and timeline with its reason.
    ///
    /// The step is created directly in [`StepStatus::Skipped`] state and the
    /// reason is stored in the output as `{"reason": "..."}`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the store fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let tests_passed = false;
    /// if tests_passed {
    ///     // ctx.shell("deploy", ...).await?;
    /// } else {
    ///     ctx.skip("deploy", "tests failed").await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn skip(&mut self, name: &str, reason: &str) -> Result<(), EngineError> {
        let position = self.position;
        self.position += 1;

        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                name: name.to_string(),
                kind: StepKind::Custom("skip".to_string()),
                position,
                input: None,
                is_error_handler: false,
            })
            .await?;

        if !self.last_step_ids.is_empty() {
            let deps: Vec<NewStepDependency> = self
                .last_step_ids
                .iter()
                .map(|&depends_on| NewStepDependency {
                    step_id: step.id,
                    depends_on,
                })
                .collect();
            self.store.create_step_dependencies(deps).await?;
        }

        let now = Utc::now();
        self.store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Skipped),
                    output: Some(serde_json::json!({"reason": reason})),
                    completed_at: Some(now),
                    ..StepUpdate::default()
                },
            )
            .await?;

        self.last_step_ids = vec![step.id];

        info!(
            run_id = %self.run_id,
            step = %name,
            reason,
            "step skipped"
        );

        Ok(())
    }

    /// Execute a custom operation step.
    ///
    /// Runs a user-defined [`Operation`] with full step lifecycle management:
    /// creates the step record, transitions to Running, executes the operation,
    /// persists the output and duration, and marks the step Completed or Failed.
    ///
    /// The operation's [`kind()`](Operation::kind) is stored as
    /// [`StepKind::Custom`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the operation fails or the store errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::operation::Operation;
    /// use ironflow_engine::error::EngineError;
    /// use serde_json::{Value, json};
    /// use std::pin::Pin;
    /// use std::future::Future;
    ///
    /// struct MyOp;
    /// impl Operation for MyOp {
    ///     fn kind(&self) -> &str { "my-service" }
    ///     fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
    ///         Box::pin(async { Ok(json!({"ok": true})) })
    ///     }
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let result = ctx.operation("call-service", &MyOp).await?;
    /// println!("output: {}", result.output);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn operation(
        &mut self,
        name: &str,
        op: &dyn Operation,
    ) -> Result<StepOutput, EngineError> {
        let kind = StepKind::Custom(op.kind().to_string());
        let position = self.position;
        self.position += 1;

        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                name: name.to_string(),
                kind,
                position,
                input: op.input(),
                is_error_handler: false,
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        let start = Instant::now();

        match op.execute().await {
            Ok(output_value) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                self.total_duration_ms += duration_ms;

                let completed_at = Utc::now();
                self.store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(output_value.clone()),
                            duration_ms: Some(duration_ms),
                            cost_usd: Some(Decimal::ZERO),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %self.run_id,
                    step = %name,
                    kind = op.kind(),
                    duration_ms,
                    "operation step completed"
                );

                self.last_step_ids = vec![step.id];

                Ok(StepOutput {
                    output: output_value,
                    duration_ms,
                    cost_usd: Decimal::ZERO,
                    input_tokens: None,
                    output_tokens: None,
                    model: None,
                    debug_messages: None,
                })
            }
            Err(err) => {
                let completed_at = Utc::now();
                if let Err(store_err) = self
                    .store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Failed),
                            error: Some(err.to_string()),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await
                {
                    error!(step_id = %step.id, error = %store_err, "failed to persist step failure");
                }

                Err(err)
            }
        }
    }

    /// Execute a sub-workflow step.
    ///
    /// Creates a child run for the named workflow handler, executes it with
    /// its own steps and lifecycle, and returns a [`StepOutput`] containing
    /// the child run ID and aggregated metrics.
    ///
    /// Requires the context to be created with
    /// `with_handler_resolver`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if no handler is registered
    /// with the given name, or if no handler resolver is available.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use serde_json::json;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// // let result = ctx.workflow(&MySubWorkflow, json!({})).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn workflow(
        &mut self,
        handler: &dyn WorkflowHandler,
        payload: Value,
    ) -> Result<StepOutput, EngineError> {
        // Guard check: verify limits before creating the step.
        if let (Some(guard_config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            let state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.check(guard_config, handler.name())?;
        }

        let config = WorkflowStepConfig::new(handler.name(), payload);
        let position = self.position;
        self.position += 1;

        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                name: config.workflow_name.clone(),
                kind: StepKind::Workflow,
                position,
                input: Some(serde_json::to_value(&config)?),
                is_error_handler: false,
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        // Record invocation in guard state (fail-closed).
        if let Some(guard_state) = &self.guard_state {
            let mut state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.record_invocation(handler.name());
        }

        match self.execute_child_workflow(&config).await {
            Ok((output, child_had_allowed_failure)) => {
                self.total_cost_usd += output.cost_usd;
                self.total_duration_ms += output.duration_ms;
                if child_had_allowed_failure {
                    self.has_allowed_failure = true;
                }

                let completed_at = Utc::now();
                self.store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(output.output.clone()),
                            duration_ms: Some(output.duration_ms),
                            cost_usd: Some(output.cost_usd),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %self.run_id,
                    child_workflow = %config.workflow_name,
                    duration_ms = output.duration_ms,
                    "workflow step completed"
                );

                self.last_step_ids = vec![step.id];

                self.guard_record_return();
                Ok(output)
            }
            Err(err) => {
                let completed_at = Utc::now();
                if let Err(store_err) = self
                    .store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Failed),
                            error: Some(err.to_string()),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await
                {
                    error!(step_id = %step.id, error = %store_err, "failed to persist step failure");
                }

                self.guard_record_return();
                Err(err)
            }
        }
    }

    /// Decrement guard state after a sub-workflow returns (success or failure).
    ///
    /// Logs on poison rather than propagating, because this runs on error
    /// paths where the workflow is already failing.
    fn guard_record_return(&self) {
        if let Some(guard_state) = &self.guard_state {
            match guard_state.lock() {
                Ok(mut state) => state.record_return(),
                Err(_) => {
                    error!(
                        run_id = %self.run_id,
                        "guard state mutex poisoned in record_return"
                    );
                }
            }
        }
    }

    /// Wrap step execution with the guard's remaining timeout.
    ///
    /// When no guard is configured the step runs without a timeout wrapper.
    async fn execute_with_guard_timeout(
        &self,
        config: &StepConfig,
        step_log_sender: Option<StepLogSender>,
    ) -> Result<StepOutput, EngineError> {
        let remaining = self.guard_remaining_timeout();
        match remaining {
            Some(dur) => {
                use tokio::time::timeout;
                match timeout(
                    dur,
                    execute_step_config(config, &self.provider, step_log_sender),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        let config_secs = self
                            .guard_config
                            .as_ref()
                            .map_or(0, |c| c.workflow_timeout_secs);
                        Err(WorkflowRejection::WorkflowTimeout {
                            elapsed_secs: config_secs,
                            max: config_secs,
                        }
                        .into())
                    }
                }
            }
            None => execute_step_config(config, &self.provider, step_log_sender).await,
        }
    }

    /// Compute the remaining timeout duration from the guard, if any.
    fn guard_remaining_timeout(&self) -> Option<std::time::Duration> {
        let config = self.guard_config.as_ref()?;
        let guard_state = self.guard_state.as_ref()?;
        let state = guard_state.lock().ok()?;
        let elapsed = state.elapsed_secs();
        let max = config.workflow_timeout_secs;
        if elapsed >= max {
            Some(std::time::Duration::ZERO)
        } else {
            Some(std::time::Duration::from_secs(max - elapsed))
        }
    }

    /// Check the guard timeout before every step.
    fn check_guard_timeout(&self) -> Result<(), EngineError> {
        if let (Some(config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            let state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            let elapsed = state.elapsed_secs();
            if elapsed >= config.workflow_timeout_secs {
                return Err(WorkflowRejection::WorkflowTimeout {
                    elapsed_secs: elapsed,
                    max: config.workflow_timeout_secs,
                }
                .into());
            }
        }
        Ok(())
    }

    /// Record token usage from an agent step in the guard state.
    fn guard_record_tokens(&self, tokens: u64) -> Result<(), EngineError> {
        if let (Some(config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            let mut state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.record_tokens(config, tokens)?;
        }
        Ok(())
    }

    /// Execute a child workflow and return aggregated output plus whether
    /// at least one `allow_failure` step failed.
    async fn execute_child_workflow(
        &self,
        config: &WorkflowStepConfig,
    ) -> Result<(StepOutput, bool), EngineError> {
        let resolver = self.handler_resolver.as_ref().ok_or_else(|| {
            EngineError::InvalidWorkflow(
                "sub-workflow requires a handler resolver (use Engine to execute)".to_string(),
            )
        })?;

        let handler = resolver(&config.workflow_name).ok_or_else(|| {
            EngineError::InvalidWorkflow(format!("no handler registered: {}", config.workflow_name))
        })?;

        // A child run inherits both the parent labels and the parent author:
        // whoever triggered the parent workflow is accountable for its children.
        let parent = self.store.get_run(self.run_id).await?;
        let (parent_labels, parent_author) =
            parent.map(|r| (r.labels, r.created_by)).unwrap_or_default();

        let child_run = self
            .store
            .create_run(NewRun {
                workflow_name: config.workflow_name.clone(),
                trigger: TriggerKind::Workflow,
                payload: config.payload.clone(),
                max_retries: 0,
                handler_version: None,
                labels: parent_labels,
                scheduled_at: None,
                created_by: parent_author,
                idempotency_key: None,
                // The child shares the parent's cap; it does not get its own budget.
                max_cost_usd: self.max_cost_usd,
            })
            .await?
            .into_run();

        let child_run_id = child_run.id;
        info!(
            parent_run_id = %self.run_id,
            child_run_id = %child_run_id,
            workflow = %config.workflow_name,
            "child run created"
        );

        self.store
            .update_run_status(child_run_id, RunStatus::Running)
            .await?;

        let run_start = Instant::now();
        let mut child_ctx = WorkflowContext {
            run_id: child_run_id,
            store: self.store.clone(),
            provider: self.provider.clone(),
            handler_resolver: self.handler_resolver.clone(),
            position: 0,
            last_step_ids: Vec::new(),
            total_cost_usd: Decimal::ZERO,
            total_duration_ms: 0,
            max_cost_usd: self.max_cost_usd,
            // Everything the parent chain already spent counts against the
            // shared cap, so the child cannot restart the budget from zero.
            inherited_cost_usd: self.charged_cost_usd(),
            replay_steps: HashMap::new(),
            granted_approvals: HashMap::new(),
            // A child run is created fresh here; it is never itself retried.
            attempt: 1,
            carried_duration_ms: 0,
            log_sender: self.log_sender.clone(),
            // A child shares the storage backend but not the parent's artifacts:
            // input lookups are scoped to the child's own run.
            artifact_sink: self.artifact_sink.clone(),
            has_allowed_failure: false,
            error_handlers: Vec::new(),
            guard_state: self.guard_state.clone(),
            guard_config: self.guard_config.clone(),
            event_bus: self.event_bus.clone(),
        };

        let result = handler.execute(&mut child_ctx).await;
        let total_duration = run_start.elapsed().as_millis() as u64;
        let completed_at = Utc::now();

        match result {
            Ok(()) => {
                let child_status = if child_ctx.has_allowed_failure {
                    RunStatus::Warning
                } else {
                    RunStatus::Completed
                };
                self.store
                    .update_run(
                        child_run_id,
                        RunUpdate {
                            status: Some(child_status),
                            cost_usd: Some(child_ctx.total_cost_usd),
                            duration_ms: Some(total_duration),
                            completed_at: Some(completed_at),
                            ..RunUpdate::default()
                        },
                    )
                    .await?;

                let child_had_allowed_failure = child_ctx.has_allowed_failure;
                Ok((
                    StepOutput {
                        output: serde_json::json!({
                            "run_id": child_run_id,
                            "workflow_name": config.workflow_name,
                            "status": child_status,
                            "cost_usd": child_ctx.total_cost_usd,
                            "duration_ms": total_duration,
                        }),
                        duration_ms: total_duration,
                        cost_usd: child_ctx.total_cost_usd,
                        input_tokens: None,
                        output_tokens: None,
                        model: None,
                        debug_messages: None,
                    },
                    child_had_allowed_failure,
                ))
            }
            Err(err) => {
                if let Err(store_err) = self
                    .store
                    .update_run(
                        child_run_id,
                        RunUpdate {
                            status: Some(RunStatus::Failed),
                            error: Some(err.to_string()),
                            cost_usd: Some(child_ctx.total_cost_usd),
                            duration_ms: Some(total_duration),
                            completed_at: Some(completed_at),
                            ..RunUpdate::default()
                        },
                    )
                    .await
                {
                    error!(
                        child_run_id = %child_run_id,
                        store_error = %store_err,
                        "failed to persist child run failure"
                    );
                }

                Err(err)
            }
        }
    }

    /// Try to replay a completed step from a previous execution.
    ///
    /// Returns `Some(StepOutput)` if a completed step exists at the given
    /// position, `None` otherwise.
    fn try_replay_step(&mut self, position: u32) -> Option<StepOutput> {
        let step = self.replay_steps.get(&position)?;
        if step.status.state != StepStatus::Completed {
            return None;
        }
        let output = StepOutput {
            output: step.output.clone().unwrap_or(Value::Null),
            duration_ms: step.duration_ms,
            cost_usd: step.cost_usd,
            input_tokens: step.input_tokens,
            output_tokens: step.output_tokens,
            model: None,
            debug_messages: None,
        };
        self.total_cost_usd += output.cost_usd;
        self.total_duration_ms += output.duration_ms;
        self.last_step_ids = vec![step.id];
        info!(
            run_id = %self.run_id,
            step = %step.name,
            position,
            "step replayed from previous execution"
        );
        Some(output)
    }

    /// Internal: execute a step with full persistence lifecycle.
    #[tracing::instrument(
        name = "context.execute_step",
        skip_all,
        fields(
            run_id = %self.run_id,
            step.name = %name,
            step.kind,
            step.position = self.position,
        )
    )]
    async fn execute_step(
        &mut self,
        name: &str,
        kind: StepKind,
        config: StepConfig,
    ) -> Result<StepOutput, EngineError> {
        let kind_str: &'static str = match kind {
            StepKind::Shell => "shell",
            StepKind::Http => "http",
            StepKind::Agent => "agent",
            StepKind::Workflow => "workflow",
            StepKind::Approval => "approval",
            StepKind::Custom(_) => "custom",
        };
        Span::current().record("step.kind", kind_str);

        // Guard timeout: checked before every step, not just sub-workflows.
        self.check_guard_timeout()?;

        let position = self.position;
        self.position += 1;

        // Replay: if this step already completed in a prior execution, return cached output.
        if let Some(output) = self.try_replay_step(position) {
            return Ok(output);
        }

        // Cost cap: refuse before creating the step record, so a run that hits
        // its cap never launches the work it cannot afford.
        if let StepConfig::Agent(ref agent_config) = config {
            self.check_run_budget(step_budget_usd(agent_config.max_budget_usd))?;
        }

        // Create step record in Pending.
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                name: name.to_string(),
                kind,
                position,
                input: Some(serde_json::to_value(&config)?),
                is_error_handler: false,
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        if let Some(ref bus) = self.event_bus {
            bus.publish(
                self.run_id,
                crate::notify::WorkflowEvent::StepStarted {
                    step_name: name.to_string(),
                    step_index: position,
                    timestamp: Utc::now(),
                },
            );
        }

        // Inputs must exist before the command runs. A failure here fails the
        // step: the command would otherwise run against missing files.
        if let Err(err) = self.prepare_step_inputs(&config, position).await {
            self.fail_step(step.id, &err).await;
            if config.allow_failure() {
                self.has_allowed_failure = true;
                self.last_step_ids = vec![step.id];
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    error = %err,
                    "step input preparation failed but allow_failure is set, continuing"
                );
                return Ok(StepOutput {
                    output: json!({"error": err.to_string()}),
                    duration_ms: 0,
                    cost_usd: Decimal::ZERO,
                    input_tokens: None,
                    output_tokens: None,
                    model: None,
                    debug_messages: None,
                });
            }
            return Err(err);
        }

        let step_log_sender = self
            .log_sender
            .as_ref()
            .map(|s| StepLogSender::new(s.clone(), self.run_id, step.id, name.to_string()));

        let execution = self
            .execute_with_guard_timeout(&config, step_log_sender)
            .await;

        let execution = self
            .retry_step_if_configured(name, kind_str, &config, step.id, execution)
            .await;

        if let Err(err) = self
            .store_step_outputs(&config, step.id, name, execution.is_ok())
            .await
        {
            self.fail_step(step.id, &err).await;
            return Err(err);
        }

        match execution {
            Ok(output) => {
                self.total_cost_usd += output.cost_usd;
                self.total_duration_ms += output.duration_ms;

                // Record token usage in the guard for agent steps.
                if matches!(config, StepConfig::Agent(_)) {
                    let tokens = output
                        .input_tokens
                        .unwrap_or(0)
                        .saturating_add(output.output_tokens.unwrap_or(0));
                    if tokens > 0 {
                        self.guard_record_tokens(tokens)?;
                    }
                }

                let debug_messages_json = output.debug_messages_json();

                let completed_at = Utc::now();
                self.store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(output.output.clone()),
                            duration_ms: Some(output.duration_ms),
                            cost_usd: Some(output.cost_usd),
                            input_tokens: output.input_tokens,
                            output_tokens: output.output_tokens,
                            completed_at: Some(completed_at),
                            debug_messages: debug_messages_json,
                            ..StepUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %self.run_id,
                    step = %name,
                    duration_ms = output.duration_ms,
                    "step completed"
                );

                if let Some(ref bus) = self.event_bus {
                    bus.publish(
                        self.run_id,
                        crate::notify::WorkflowEvent::StepCompleted {
                            step_name: name.to_string(),
                            step_index: position,
                            duration_ms: output.duration_ms,
                            output_summary: None,
                        },
                    );
                }

                self.last_step_ids = vec![step.id];

                Ok(output)
            }
            Err(err) => {
                let completed_at = Utc::now();
                let debug_messages_json = extract_debug_messages_from_error(&err);
                let partial = extract_partial_usage_from_error(&err);
                let raw_response_output = extract_raw_response_from_error(&err);

                if let Some(ref usage) = partial {
                    if let Some(cost) = usage.cost_usd {
                        self.total_cost_usd += cost;
                    }
                    if let Some(dur) = usage.duration_ms {
                        self.total_duration_ms += dur;
                    }
                }

                if let Err(store_err) = self
                    .store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Failed),
                            error: Some(err.to_string()),
                            output: raw_response_output.clone(),
                            completed_at: Some(completed_at),
                            debug_messages: debug_messages_json,
                            duration_ms: partial.as_ref().and_then(|p| p.duration_ms),
                            cost_usd: partial.as_ref().and_then(|p| p.cost_usd),
                            input_tokens: partial.as_ref().and_then(|p| p.input_tokens),
                            output_tokens: partial.as_ref().and_then(|p| p.output_tokens),
                            ..StepUpdate::default()
                        },
                    )
                    .await
                {
                    tracing::error!(step_id = %step.id, error = %store_err, "failed to persist step failure");
                }

                let err_duration = partial.as_ref().and_then(|p| p.duration_ms).unwrap_or(0);

                if let Some(ref bus) = self.event_bus {
                    bus.publish(
                        self.run_id,
                        crate::notify::WorkflowEvent::StepFailed {
                            step_name: name.to_string(),
                            step_index: position,
                            error: err.to_string(),
                            duration_ms: err_duration,
                        },
                    );
                }

                self.fire_error_handlers(name, &err.to_string(), err_duration)
                    .await;

                if config.allow_failure() {
                    self.has_allowed_failure = true;
                    self.last_step_ids = vec![step.id];
                    info!(
                        run_id = %self.run_id,
                        step = %name,
                        error = %err,
                        "step failed but allow_failure is set, continuing"
                    );
                    Ok(allowed_failure_output(
                        &err.to_string(),
                        raw_response_output,
                        partial.as_ref(),
                    ))
                } else {
                    Err(err)
                }
            }
        }
    }

    /// Retry a failed step execution when a step-level retry policy is configured
    /// and the error is transient.
    ///
    /// Returns the original result unchanged when no retry policy is set, the
    /// first attempt succeeded, or the error is not retryable.
    #[cfg_attr(not(feature = "prometheus"), allow(unused_variables))]
    async fn retry_step_if_configured(
        &self,
        name: &str,
        kind_str: &str,
        config: &StepConfig,
        step_id: Uuid,
        first_result: Result<StepOutput, EngineError>,
    ) -> Result<StepOutput, EngineError> {
        let policy = match config.retry() {
            Some(p) => p,
            None => return first_result,
        };

        let mut last_result = match first_result {
            Ok(output) => return Ok(output),
            Err(err) if !is_step_retryable(&err) => return Err(err),
            Err(err) => Err(err),
        };

        let step_log_sender = self
            .log_sender
            .as_ref()
            .map(|s| StepLogSender::new(s.clone(), self.run_id, step_id, name.to_string()));

        for attempt in 0..policy.max_retries() {
            if let StepConfig::Agent(agent_config) = config {
                self.check_run_budget(step_budget_usd(agent_config.max_budget_usd))?;
            }

            let delay = policy.delay_for_attempt(attempt);
            info!(
                run_id = %self.run_id,
                step = %name,
                attempt = attempt + 1,
                max_retries = policy.max_retries(),
                delay_ms = delay.as_millis() as u64,
                "retrying step after transient failure"
            );
            tokio::time::sleep(delay).await;

            record_retry_metric(kind_str, "retry");

            match execute_step_config(config, &self.provider, step_log_sender.clone()).await {
                Ok(output) => return Ok(output),
                Err(err) if !is_step_retryable(&err) => return Err(err),
                err => last_result = err,
            }
        }

        record_retry_metric(kind_str, "exhausted");

        info!(
            run_id = %self.run_id,
            step = %name,
            max_retries = policy.max_retries(),
            "step retries exhausted"
        );

        last_result
    }

    /// Record dependency edges and transition a step to Running.
    ///
    /// Records edges from `step_id` to all `last_step_ids`, then
    /// transitions the step to `Running` with the given timestamp.
    async fn start_step(&self, step_id: Uuid, now: DateTime<Utc>) -> Result<(), EngineError> {
        if !self.last_step_ids.is_empty() {
            let deps: Vec<NewStepDependency> = self
                .last_step_ids
                .iter()
                .map(|&depends_on| NewStepDependency {
                    step_id,
                    depends_on,
                })
                .collect();
            self.store.create_step_dependencies(deps).await?;
        }

        self.store
            .update_step(
                step_id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    started_at: Some(now),
                    ..StepUpdate::default()
                },
            )
            .await?;

        Ok(())
    }

    /// Mark a step as failed, best-effort.
    ///
    /// Used on paths that fail around the operation itself (artifact inputs and
    /// outputs), where the step record is already `Running` and the caller is
    /// about to propagate `err`. A store failure here is logged, never returned:
    /// it must not replace the error the caller is reporting.
    async fn fail_step(&self, step_id: Uuid, err: &EngineError) {
        if let Err(store_err) = self
            .store
            .update_step(
                step_id,
                StepUpdate {
                    status: Some(StepStatus::Failed),
                    error: Some(err.to_string()),
                    completed_at: Some(Utc::now()),
                    ..StepUpdate::default()
                },
            )
            .await
        {
            error!(
                step_id = %step_id,
                error = %store_err,
                "failed to persist step failure"
            );
        }
    }

    /// Access the store directly (advanced usage).
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Access the payload that triggered this run.
    ///
    /// Fetches the run from the store and returns its payload.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] if the run is not found.
    pub async fn payload(&self) -> Result<Value, EngineError> {
        let run = self
            .store
            .get_run(self.run_id)
            .await?
            .ok_or(EngineError::Store(
                ironflow_store::error::StoreError::RunNotFound(self.run_id),
            ))?;
        Ok(run.payload)
    }

    /// Deserialize the run payload into a typed input struct.
    ///
    /// Shorthand for `serde_json::from_value(ctx.payload().await?)`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] if the run is not found, or
    /// [`EngineError::Serialization`] if the payload does not match `T`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use ironflow_engine::context::WorkflowContext;
    /// # use ironflow_engine::error::EngineError;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct DeployInput {
    ///     environment: String,
    ///     dry_run: Option<bool>,
    /// }
    ///
    /// # async fn example(ctx: &WorkflowContext) -> Result<(), EngineError> {
    /// let input: DeployInput = ctx.input().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn input<T: serde::de::DeserializeOwned>(&self) -> Result<T, EngineError> {
        let payload = self.payload().await?;
        serde_json::from_value(payload).map_err(EngineError::Serialization)
    }

    /// Register an error handler that fires when any subsequent step fails.
    ///
    /// The handler is consumed after firing (fire-once). Multiple handlers
    /// can be registered; they fire in registration order.
    ///
    /// Error handler execution is best-effort: if a handler fails, the error
    /// is logged but the original step error is preserved. Error handler steps
    /// appear in the run timeline with [`Step::is_error_handler`] set to `true`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// ctx.on_error("cleanup", ShellConfig::new("rm -rf /tmp/build"));
    /// ctx.shell("build", ShellConfig::new("cargo build")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_error(&mut self, name: &str, config: impl Into<StepConfig>) {
        self.error_handlers.push(OnErrorHandler {
            name: name.to_string(),
            config: config.into(),
        });
    }

    /// Remove all registered error handlers.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// ctx.on_error("cleanup", ShellConfig::new("rm -rf /tmp/build"));
    /// ctx.shell("build", ShellConfig::new("cargo build")).await?;
    /// ctx.clear_error_handlers();
    /// // cleanup will NOT fire if deploy fails
    /// ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn clear_error_handlers(&mut self) {
        self.error_handlers.clear();
    }

    /// Execute all registered error handlers after a step failure.
    ///
    /// Drains the handler list (fire-once). Each handler creates its own
    /// step record with `is_error_handler = true`. Handler failures are
    /// logged but never propagated.
    async fn fire_error_handlers(
        &mut self,
        failed_step_name: &str,
        error_msg: &str,
        duration_ms: u64,
    ) {
        let handlers = std::mem::take(&mut self.error_handlers);
        if handlers.is_empty() {
            return;
        }

        let error_context = json!({
            "failed_step": failed_step_name,
            "error": error_msg,
            "duration_ms": duration_ms,
        });

        for handler in handlers {
            let mut config = handler.config.clone();
            inject_error_context(&mut config, failed_step_name, error_msg, duration_ms);

            let position = self.position;
            self.position += 1;

            let step = match self
                .store
                .create_step(NewStep {
                    run_id: self.run_id,
                    name: handler.name.clone(),
                    kind: config.kind(),
                    position,
                    input: Some(error_context.clone()),
                    is_error_handler: true,
                })
                .await
            {
                Ok(step) => step,
                Err(err) => {
                    warn!(
                        run_id = %self.run_id,
                        handler = %handler.name,
                        error = %err,
                        "failed to create error handler step"
                    );
                    continue;
                }
            };

            if let Err(err) = self.start_step(step.id, Utc::now()).await {
                warn!(
                    run_id = %self.run_id,
                    handler = %handler.name,
                    error = %err,
                    "failed to start error handler step"
                );
                continue;
            }

            let step_log_sender = self
                .log_sender
                .as_ref()
                .map(|s| StepLogSender::new(s.clone(), self.run_id, step.id, handler.name.clone()));

            let start = Instant::now();
            let result = execute_step_config(&config, &self.provider, step_log_sender).await;
            let handler_duration = start.elapsed().as_millis() as u64;
            let completed_at = Utc::now();

            match result {
                Ok(output) => {
                    if let Err(store_err) = self
                        .store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Completed),
                                output: Some(output.output),
                                duration_ms: Some(handler_duration),
                                cost_usd: Some(output.cost_usd),
                                completed_at: Some(completed_at),
                                ..StepUpdate::default()
                            },
                        )
                        .await
                    {
                        warn!(
                            run_id = %self.run_id,
                            handler = %handler.name,
                            error = %store_err,
                            "failed to persist error handler completion"
                        );
                    }

                    info!(
                        run_id = %self.run_id,
                        handler = %handler.name,
                        duration_ms = handler_duration,
                        "error handler completed"
                    );
                }
                Err(err) => {
                    if let Err(store_err) = self
                        .store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Failed),
                                error: Some(err.to_string()),
                                duration_ms: Some(handler_duration),
                                completed_at: Some(completed_at),
                                ..StepUpdate::default()
                            },
                        )
                        .await
                    {
                        warn!(
                            run_id = %self.run_id,
                            handler = %handler.name,
                            error = %store_err,
                            "failed to persist error handler failure"
                        );
                    }

                    warn!(
                        run_id = %self.run_id,
                        handler = %handler.name,
                        error = %err,
                        "error handler failed (original error preserved)"
                    );
                }
            }
        }
    }
}

/// Inject error context into a step config before executing it as an error handler.
fn inject_error_context(
    config: &mut StepConfig,
    failed_step: &str,
    error_msg: &str,
    duration_ms: u64,
) {
    match config {
        StepConfig::Shell(shell) => {
            shell
                .env
                .push(("IRONFLOW_ERROR_STEP".to_string(), failed_step.to_string()));
            shell
                .env
                .push(("IRONFLOW_ERROR_MESSAGE".to_string(), error_msg.to_string()));
            shell.env.push((
                "IRONFLOW_ERROR_DURATION_MS".to_string(),
                duration_ms.to_string(),
            ));
        }
        StepConfig::Agent(agent) => {
            agent.prompt = format!(
                "[Error Context]\nStep \"{}\" failed after {}ms:\n{}\n\n{}",
                failed_step, duration_ms, error_msg, agent.prompt
            );
        }
        StepConfig::Http(http) => {
            http.headers
                .push(("X-Ironflow-Error-Step".to_string(), failed_step.to_string()));
            http.headers.push((
                "X-Ironflow-Error-Message".to_string(),
                error_msg.to_string(),
            ));
        }
        StepConfig::Workflow(_) | StepConfig::Approval(_) => {}
    }
}

#[cfg(feature = "prometheus")]
fn record_retry_metric(kind: &str, outcome: &str) {
    use ironflow_core::metric_names::STEP_RETRIES_TOTAL;
    use metrics::counter;
    counter!(STEP_RETRIES_TOTAL, "kind" => kind.to_string(), "outcome" => outcome.to_string())
        .increment(1);
}

#[cfg(not(feature = "prometheus"))]
fn record_retry_metric(_kind: &str, _outcome: &str) {}

/// Step-level retryability: broader than operation-level retry because the user
/// explicitly opted in. Excludes only deterministic or financially wasteful
/// errors that retrying cannot fix.
fn is_step_retryable(err: &EngineError) -> bool {
    use ironflow_core::error::{AgentError, OperationError};

    match err {
        EngineError::Operation(op) => match op {
            OperationError::Agent(AgentError::PromptTooLarge { .. }) => false,
            OperationError::Agent(AgentError::BudgetExceeded { .. }) => false,
            OperationError::Deserialize { .. } => false,
            OperationError::Http {
                status: Some(code), ..
            } if (400..500).contains(code) && *code != 429 => false,
            _ => true,
        },
        _ => false,
    }
}

fn allowed_failure_output(
    error_msg: &str,
    raw_response: Option<Value>,
    partial: Option<&StepPartialUsage>,
) -> StepOutput {
    StepOutput {
        output: raw_response.unwrap_or_else(|| json!({"error": error_msg})),
        duration_ms: partial.and_then(|p| p.duration_ms).unwrap_or(0),
        cost_usd: partial.and_then(|p| p.cost_usd).unwrap_or(Decimal::ZERO),
        input_tokens: partial.and_then(|p| p.input_tokens),
        output_tokens: partial.and_then(|p| p.output_tokens),
        model: None,
        debug_messages: None,
    }
}

impl fmt::Debug for WorkflowContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkflowContext")
            .field("run_id", &self.run_id)
            .field("position", &self.position)
            .field("total_cost_usd", &self.total_cost_usd)
            .field("inherited_cost_usd", &self.inherited_cost_usd)
            .field("max_cost_usd", &self.max_cost_usd)
            .finish_non_exhaustive()
    }
}

/// Extract debug messages from an engine error, if it wraps a schema validation
/// failure that carries a verbose conversation trace.
fn extract_debug_messages_from_error(err: &EngineError) -> Option<Value> {
    if let EngineError::Operation(OperationError::Agent(AgentError::SchemaValidation {
        debug_messages,
        ..
    })) = err
        && !debug_messages.is_empty()
    {
        return serde_json::to_value(debug_messages).ok();
    }
    None
}

/// Partial usage with `Decimal` cost, converted from the `f64` in [`PartialUsage`].
///
/// Exists only because `ironflow-store` uses [`Decimal`] for monetary values
/// while `ironflow-core` uses `f64` (the CLI's native type). The conversion
/// happens here, at the engine/store boundary.
struct StepPartialUsage {
    cost_usd: Option<Decimal>,
    duration_ms: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

/// Extract the raw response text from a schema validation error.
///
/// When the agent produced text but structured output extraction failed,
/// this returns the truncated raw text so it can be persisted as the
/// step output for dashboard visibility.
fn extract_raw_response_from_error(err: &EngineError) -> Option<Value> {
    if let EngineError::Operation(OperationError::Agent(AgentError::SchemaValidation {
        raw_response: Some(text),
        ..
    })) = err
    {
        return Some(Value::String(text.clone()));
    }
    None
}

fn extract_partial_usage_from_error(err: &EngineError) -> Option<StepPartialUsage> {
    if let EngineError::Operation(OperationError::Agent(AgentError::SchemaValidation {
        partial_usage,
        ..
    })) = err
        && (partial_usage.cost_usd.is_some() || partial_usage.duration_ms.is_some())
    {
        return Some(StepPartialUsage {
            cost_usd: partial_usage
                .cost_usd
                .and_then(|c| Decimal::try_from(c).ok()),
            duration_ms: partial_usage.duration_ms,
            input_tokens: partial_usage.input_tokens,
            output_tokens: partial_usage.output_tokens,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_core::providers::record_replay::RecordReplayProvider;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{Run, RunActor, RunFilter};
    use ironflow_store::store::RunStore;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use uuid::Uuid;

    /// Helper to create a test provider with fixtures
    fn create_test_provider() -> Arc<dyn ironflow_core::provider::AgentProvider> {
        let inner = ClaudeCodeProvider::new();
        Arc::new(RecordReplayProvider::replay(
            inner,
            "/tmp/ironflow-fixtures",
        ))
    }

    /// Helper to create a test context
    fn create_test_context() -> WorkflowContext {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();
        let run_id = Uuid::now_v7();
        WorkflowContext::new(run_id, store, provider)
    }

    #[test]
    fn context_new_initializes_correctly() {
        let ctx = create_test_context();
        assert_eq!(ctx.position, 0);
        assert_eq!(ctx.total_cost_usd, Decimal::ZERO);
        assert_eq!(ctx.total_duration_ms, 0);
        assert!(ctx.last_step_ids.is_empty());
        assert!(ctx.replay_steps.is_empty());
        assert!(ctx.log_sender.is_none());
    }

    #[test]
    fn context_run_id_returns_correct_id() {
        let run_id = Uuid::now_v7();
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();
        let ctx = WorkflowContext::new(run_id, store, provider);
        assert_eq!(ctx.run_id(), run_id);
    }

    #[test]
    fn context_total_cost_usd_initially_zero() {
        let ctx = create_test_context();
        assert_eq!(ctx.total_cost_usd(), Decimal::ZERO);
    }

    #[test]
    fn context_total_duration_ms_initially_zero() {
        let ctx = create_test_context();
        assert_eq!(ctx.total_duration_ms(), 0);
    }

    #[test]
    fn context_with_handler_resolver_creates_context_with_resolver() {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();
        let run_id = Uuid::now_v7();

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        let resolver: HandlerResolver = Arc::new(move |_name: &str| {
            called_clone.store(true, Ordering::SeqCst);
            None
        });

        let ctx = WorkflowContext::with_handler_resolver(run_id, store, provider, resolver);

        assert_eq!(ctx.run_id(), run_id);
        assert!(ctx.handler_resolver.is_some());
    }

    #[tokio::test]
    async fn context_set_log_sender_attaches_sender() {
        let mut ctx = create_test_context();
        let (sender, _receiver) = crate::log_sender::channel();
        ctx.set_log_sender(sender);
        assert!(ctx.log_sender.is_some());
    }

    #[tokio::test]
    async fn context_skip_creates_skipped_step() {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();

        // Create the run first using RunStore trait
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("failed to create run")
            .into_run();

        // Get the created run to extract its ID
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("failed to list runs");
        let created_run_id = runs.items[0].id;

        let mut ctx = WorkflowContext::new(created_run_id, store.clone(), provider);
        let initial_position = ctx.position;

        ctx.skip("skip-step", "condition not met")
            .await
            .expect("skip failed");

        assert_eq!(ctx.position, initial_position + 1);
        assert!(!ctx.last_step_ids.is_empty());

        // Verify the step was recorded with Skipped status
        let steps = store
            .list_steps(created_run_id)
            .await
            .expect("failed to list steps");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].status.state, StepStatus::Skipped);
    }

    /// Sub-workflow handler that records no steps, so the child run reaches a
    /// terminal state without touching the filesystem or the network.
    struct NoopSubWorkflow;

    impl WorkflowHandler for NoopSubWorkflow {
        fn name(&self) -> &str {
            "noop-sub"
        }

        fn execute<'a>(
            &'a self,
            _ctx: &'a mut WorkflowContext,
        ) -> crate::handler::HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    /// Run a parent workflow authored by `created_by` and return the child run
    /// created by its sub-workflow step.
    async fn child_run_of_parent_authored_by(created_by: Option<RunActor>) -> Run {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();

        let parent = store
            .create_run(NewRun {
                workflow_name: "parent".to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                created_by,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("failed to create parent run")
            .into_run();

        let resolver: HandlerResolver = Arc::new(|name: &str| match name {
            "noop-sub" => Some(Arc::new(NoopSubWorkflow) as Arc<dyn WorkflowHandler>),
            _ => None,
        });

        let mut ctx =
            WorkflowContext::with_handler_resolver(parent.id, store.clone(), provider, resolver);
        ctx.workflow(&NoopSubWorkflow, json!({}))
            .await
            .expect("sub-workflow failed");

        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("failed to list runs");
        runs.items
            .into_iter()
            .find(|r| r.workflow_name == "noop-sub")
            .expect("child run was created")
    }

    #[tokio::test]
    async fn child_run_inherits_the_parent_author() {
        let user_id = Uuid::now_v7();
        let child = child_run_of_parent_authored_by(Some(RunActor::User { user_id })).await;

        assert_eq!(child.created_by, Some(RunActor::User { user_id }));
    }

    #[tokio::test]
    async fn child_run_of_an_unattributed_parent_has_no_author() {
        let child = child_run_of_parent_authored_by(None).await;

        assert!(child.created_by.is_none());
    }

    #[tokio::test]
    async fn context_parallel_empty_steps_returns_empty_vec() {
        let mut ctx = create_test_context();
        let results = ctx
            .parallel(vec![], true)
            .await
            .expect("parallel should not fail on empty input");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn context_approval_first_execution_returns_error() {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();

        // Create the run first
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("failed to create run")
            .into_run();

        // Get the created run to extract its ID
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("failed to list runs");
        let created_run_id = runs.items[0].id;

        let mut ctx = WorkflowContext::new(created_run_id, store.clone(), provider);

        let result = ctx
            .approval(
                "approve-step",
                crate::config::ApprovalConfig::new("Continue?"),
            )
            .await;

        // First execution should return ApprovalRequired error
        assert!(matches!(result, Err(EngineError::ApprovalRequired { .. })));

        // Verify position incremented
        assert_eq!(ctx.position, 1);

        // Verify step was created with AwaitingApproval status
        let steps = store
            .list_steps(created_run_id)
            .await
            .expect("failed to list steps");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].status.state, StepStatus::AwaitingApproval);
    }

    #[tokio::test]
    async fn context_approval_replay_returns_ok() {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();

        // Create the run first
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("failed to create run")
            .into_run();

        // Get the created run to extract its ID
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("failed to list runs");
        let created_run_id = runs.items[0].id;

        // Create an approval step that's already in AwaitingApproval state
        let step = store
            .create_step(NewStep {
                run_id: created_run_id,
                name: "approval".to_string(),
                kind: StepKind::Approval,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("failed to create step");

        // Transition through proper states: Pending -> Running -> AwaitingApproval
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
            .expect("failed to update step to Running");

        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("failed to update step to AwaitingApproval");

        // Create context and load replay steps
        let mut ctx = WorkflowContext::new(created_run_id, store.clone(), provider);
        ctx.load_replay_steps()
            .await
            .expect("failed to load replay steps");

        // Now approval should succeed (replay)
        let result = ctx
            .approval("approval", crate::config::ApprovalConfig::new("Continue?"))
            .await;

        assert!(result.is_ok());

        // Verify the step was marked Completed
        let steps = store
            .list_steps(created_run_id)
            .await
            .expect("failed to list steps");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].status.state, StepStatus::Completed);
    }

    #[tokio::test]
    async fn context_load_replay_steps_loads_completed_steps() {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();

        // Create the run first
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("failed to create run")
            .into_run();

        // Get the created run to extract its ID
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("failed to list runs");
        let created_run_id = runs.items[0].id;

        // Create multiple steps with different statuses
        let completed_step = store
            .create_step(NewStep {
                run_id: created_run_id,
                name: "completed".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("failed to create step");

        // Transition to Running then Completed
        store
            .update_step(
                completed_step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    started_at: Some(Utc::now()),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("failed to update step to Running");

        store
            .update_step(
                completed_step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    completed_at: Some(Utc::now()),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("failed to update step to Completed");

        let _pending_step = store
            .create_step(NewStep {
                run_id: created_run_id,
                name: "pending".to_string(),
                kind: StepKind::Shell,
                position: 1,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("failed to create step");

        // Load replay steps
        let mut ctx = WorkflowContext::new(created_run_id, store, provider);
        ctx.load_replay_steps()
            .await
            .expect("failed to load replay steps");

        // Only completed step should be in replay_steps
        assert_eq!(ctx.replay_steps.len(), 1);
        assert!(ctx.replay_steps.contains_key(&0));
        assert!(!ctx.replay_steps.contains_key(&1));
    }

    #[tokio::test]
    async fn context_payload_returns_run_payload() {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();
        let test_payload = json!({"key": "value", "number": 42});

        // Create the run first
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: test_payload.clone(),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("failed to create run")
            .into_run();

        // Get the created run to extract its ID
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("failed to list runs");
        let created_run_id = runs.items[0].id;

        let ctx = WorkflowContext::new(created_run_id, store, provider);
        let payload = ctx.payload().await.expect("failed to get payload");

        assert_eq!(payload, test_payload);
    }

    #[tokio::test]
    async fn context_payload_returns_error_for_nonexistent_run() {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();
        let run_id = Uuid::now_v7();

        let ctx = WorkflowContext::new(run_id, store, provider);
        let result = ctx.payload().await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn context_store_returns_reference() {
        let ctx = create_test_context();
        let _store = ctx.store();
        // store() returns a reference to the Arc<dyn Store>, which is always available
    }

    #[test]
    fn context_debug_formatting() {
        let ctx = create_test_context();
        let debug_str = format!("{:?}", ctx);
        assert!(debug_str.contains("WorkflowContext"));
        assert!(debug_str.contains("run_id"));
    }

    #[tokio::test]
    async fn context_last_step_ids_tracks_executed_steps() {
        let store = Arc::new(InMemoryStore::new());
        let provider = create_test_provider();

        // Create the run first
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("failed to create run")
            .into_run();

        // Get the created run to extract its ID
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("failed to list runs");
        let created_run_id = runs.items[0].id;

        let mut ctx = WorkflowContext::new(created_run_id, store, provider);
        assert!(ctx.last_step_ids.is_empty());

        ctx.skip("step1", "reason").await.expect("skip failed");

        assert_eq!(ctx.last_step_ids.len(), 1);

        ctx.skip("step2", "reason").await.expect("skip failed");

        // last_step_ids should now contain only step2's ID
        assert_eq!(ctx.last_step_ids.len(), 1);
    }
}
