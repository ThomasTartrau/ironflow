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
use rust_decimal::Decimal;
use serde_json::Value;
use tokio::task::JoinSet;
use tracing::{error, info};
use uuid::Uuid;

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::provider::AgentProvider;
use ironflow_store::models::{
    NewRun, NewStep, NewStepDependency, RunStatus, RunUpdate, Step, StepKind, StepStatus,
    StepUpdate, TriggerKind,
};
use ironflow_store::store::Store;

use crate::config::{
    AgentStepConfig, ApprovalConfig, HttpConfig, ShellConfig, StepConfig, WorkflowStepConfig,
};
use crate::error::EngineError;
use crate::executor::{ParallelStepResult, StepOutput, execute_step_config};
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
    /// Steps from a previous execution, keyed by position.
    /// Used when resuming after approval to replay completed steps.
    replay_steps: HashMap<u32, Step>,
    /// Optional sender for real-time log streaming.
    log_sender: Option<LogSender>,
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
            replay_steps: HashMap::new(),
            log_sender: None,
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
            replay_steps: HashMap::new(),
            log_sender: None,
        }
    }

    /// Attach a log sender for real-time step output streaming.
    pub fn set_log_sender(&mut self, sender: LogSender) {
        self.log_sender = Some(sender);
    }

    /// Load existing steps from the store for replay after approval.
    ///
    /// Called by the engine when resuming a run. All completed steps
    /// and the approved approval step are indexed by position so that
    /// `execute_step` and `approval` can skip them.
    pub(crate) async fn load_replay_steps(&mut self) -> Result<(), EngineError> {
        let steps = self.store.list_steps(self.run_id).await?;
        for step in steps {
            let dominated = matches!(
                step.status.state,
                StepStatus::Completed | StepStatus::Running | StepStatus::AwaitingApproval
            );
            if dominated {
                self.replay_steps.insert(step.position, step);
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
                })
                .await?;

            self.start_step(step.id, now).await?;

            step_records.push((step.id, name.to_string(), config.clone()));
        }

        let mut join_set = JoinSet::new();
        for (idx, (step_id, step_name, config)) in step_records.iter().enumerate() {
            let provider = self.provider.clone();
            let config = config.clone();
            let step_log_sender = self
                .log_sender
                .as_ref()
                .map(|s| StepLogSender::new(s.clone(), self.run_id, *step_id, step_name.clone()));
            join_set.spawn(async move {
                (
                    idx,
                    execute_step_config(&config, &provider, step_log_sender).await,
                )
            });
        }

        // JoinSet returns in completion order; indexed_results restores input order.
        let mut indexed_results: Vec<Option<Result<StepOutput, String>>> =
            vec![None; step_records.len()];
        let mut first_error: Option<EngineError> = None;

        while let Some(join_result) = join_set.join_next().await {
            let (idx, step_result) = match join_result {
                Ok(r) => r,
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(EngineError::StepConfig(format!("join error: {e}")));
                    }
                    if fail_fast {
                        join_set.abort_all();
                    }
                    continue;
                }
            };

            let (step_id, step_name, _) = &step_records[idx];
            let completed_at = Utc::now();

            match step_result {
                Ok(output) => {
                    self.total_cost_usd += output.cost_usd;
                    self.total_duration_ms += output.duration_ms;

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

        // First execution: create the approval step and suspend.
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                name: name.to_string(),
                kind: StepKind::Approval,
                position,
                input: Some(serde_json::to_value(&config)?),
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
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        match self.execute_child_workflow(&config).await {
            Ok(output) => {
                self.total_cost_usd += output.cost_usd;
                self.total_duration_ms += output.duration_ms;

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

                Err(err)
            }
        }
    }

    /// Execute a child workflow and return aggregated output.
    async fn execute_child_workflow(
        &self,
        config: &WorkflowStepConfig,
    ) -> Result<StepOutput, EngineError> {
        let resolver = self.handler_resolver.as_ref().ok_or_else(|| {
            EngineError::InvalidWorkflow(
                "sub-workflow requires a handler resolver (use Engine to execute)".to_string(),
            )
        })?;

        let handler = resolver(&config.workflow_name).ok_or_else(|| {
            EngineError::InvalidWorkflow(format!("no handler registered: {}", config.workflow_name))
        })?;

        let parent_labels = self
            .store
            .get_run(self.run_id)
            .await?
            .map(|r| r.labels)
            .unwrap_or_default();

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
            })
            .await?;

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
            replay_steps: HashMap::new(),
            log_sender: self.log_sender.clone(),
        };

        let result = handler.execute(&mut child_ctx).await;
        let total_duration = run_start.elapsed().as_millis() as u64;
        let completed_at = Utc::now();

        match result {
            Ok(()) => {
                self.store
                    .update_run(
                        child_run_id,
                        RunUpdate {
                            status: Some(RunStatus::Completed),
                            cost_usd: Some(child_ctx.total_cost_usd),
                            duration_ms: Some(total_duration),
                            completed_at: Some(completed_at),
                            ..RunUpdate::default()
                        },
                    )
                    .await?;

                Ok(StepOutput {
                    output: serde_json::json!({
                        "run_id": child_run_id,
                        "workflow_name": config.workflow_name,
                        "status": RunStatus::Completed,
                        "cost_usd": child_ctx.total_cost_usd,
                        "duration_ms": total_duration,
                    }),
                    duration_ms: total_duration,
                    cost_usd: child_ctx.total_cost_usd,
                    input_tokens: None,
                    output_tokens: None,
                    model: None,
                    debug_messages: None,
                })
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
    async fn execute_step(
        &mut self,
        name: &str,
        kind: StepKind,
        config: StepConfig,
    ) -> Result<StepOutput, EngineError> {
        let position = self.position;
        self.position += 1;

        // Replay: if this step already completed in a prior execution, return cached output.
        if let Some(output) = self.try_replay_step(position) {
            return Ok(output);
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
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        let step_log_sender = self
            .log_sender
            .as_ref()
            .map(|s| StepLogSender::new(s.clone(), self.run_id, step.id, name.to_string()));

        match execute_step_config(&config, &self.provider, step_log_sender).await {
            Ok(output) => {
                self.total_cost_usd += output.cost_usd;
                self.total_duration_ms += output.duration_ms;

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

                self.last_step_ids = vec![step.id];

                Ok(output)
            }
            Err(err) => {
                let completed_at = Utc::now();
                let debug_messages_json = extract_debug_messages_from_error(&err);
                let partial = extract_partial_usage_from_error(&err);

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

                Err(err)
            }
        }
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
}

impl fmt::Debug for WorkflowContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkflowContext")
            .field("run_id", &self.run_id)
            .field("position", &self.position)
            .field("total_cost_usd", &self.total_cost_usd)
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
