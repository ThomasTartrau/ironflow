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

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::Value;
use tracing::{error, info};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_store::models::{
    NewRun, NewStep, NewStepDependency, RunStatus, RunUpdate, StepKind, StepStatus, StepUpdate,
    TriggerKind,
};
use ironflow_store::store::RunStore;

use crate::config::{AgentStepConfig, HttpConfig, ShellConfig, StepConfig, WorkflowStepConfig};
use crate::error::EngineError;
use crate::executor::{ParallelStepResult, StepOutput, execute_step_config};
use crate::handler::WorkflowHandler;
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
    store: Arc<dyn RunStore>,
    provider: Arc<dyn AgentProvider>,
    handler_resolver: Option<HandlerResolver>,
    position: u32,
    /// IDs of the last executed step(s) -- used to record DAG dependencies.
    last_step_ids: Vec<Uuid>,
    /// Accumulated cost across all steps in this run.
    total_cost_usd: Decimal,
    /// Accumulated duration across all steps.
    total_duration_ms: u64,
}

impl WorkflowContext {
    /// Create a new context for a run.
    ///
    /// Not typically called directly — the [`Engine`](crate::engine::Engine)
    /// creates this when executing a [`WorkflowHandler`](crate::handler::WorkflowHandler).
    pub fn new(run_id: Uuid, store: Arc<dyn RunStore>, provider: Arc<dyn AgentProvider>) -> Self {
        Self {
            run_id,
            store,
            provider,
            handler_resolver: None,
            position: 0,
            last_step_ids: Vec::new(),
            total_cost_usd: Decimal::ZERO,
            total_duration_ms: 0,
        }
    }

    /// Create a new context with a handler resolver for sub-workflow support.
    ///
    /// The resolver is called when [`workflow`](Self::workflow) is invoked to
    /// look up registered handlers by name.
    pub(crate) fn with_handler_resolver(
        run_id: Uuid,
        store: Arc<dyn RunStore>,
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
        }
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

        // Create all step records and transition to Running.
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

            // Record dependencies on previous step(s).
            self.record_dependencies_for(step.id).await?;

            self.store
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Running),
                        started_at: Some(now),
                        ..StepUpdate::default()
                    },
                )
                .await?;

            step_records.push((step.id, name.to_string(), config.clone()));
        }

        // Execute all steps in parallel.
        let mut join_set = tokio::task::JoinSet::new();
        for (idx, (_id, _name, config)) in step_records.iter().enumerate() {
            let provider = self.provider.clone();
            let config = config.clone();
            join_set.spawn(async move { (idx, execute_step_config(&config, &provider).await) });
        }

        // Collect results in original order.
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

                    if let Err(store_err) = self
                        .store
                        .update_step(
                            *step_id,
                            StepUpdate {
                                status: Some(StepStatus::Failed),
                                error: Some(err_msg.clone()),
                                completed_at: Some(completed_at),
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

        // Update last_step_ids to all steps in this batch.
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
    /// println!("review: {}", review.output["value"]);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn agent(
        &mut self,
        name: &str,
        config: AgentStepConfig,
    ) -> Result<StepOutput, EngineError> {
        self.execute_step(name, StepKind::Agent, StepConfig::Agent(config))
            .await
    }

    /// Execute a custom operation step.
    ///
    /// Runs a user-defined [`Operation`] with full step lifecycle management:
    /// creates the step record, transitions to Running, executes the operation,
    /// persists the output and duration, and marks the step Completed or Failed.
    ///
    /// The operation's [`kind()`](Operation::kind) is stored as
    /// [`StepKind::Custom`](ironflow_store::models::StepKind::Custom).
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

        // Record dependencies on previous step(s).
        self.record_dependencies_for(step.id).await?;

        let now = Utc::now();
        self.store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    started_at: Some(now),
                    ..StepUpdate::default()
                },
            )
            .await?;

        let start = std::time::Instant::now();

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
    /// [`with_handler_resolver`](Self::with_handler_resolver).
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

        // Record dependencies on previous step(s).
        self.record_dependencies_for(step.id).await?;

        let now = Utc::now();
        self.store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    started_at: Some(now),
                    ..StepUpdate::default()
                },
            )
            .await?;

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

        let child_run = self
            .store
            .create_run(NewRun {
                workflow_name: config.workflow_name.clone(),
                trigger: TriggerKind::Workflow,
                payload: config.payload.clone(),
                max_retries: 0,
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

    /// Internal: execute a step with full persistence lifecycle.
    async fn execute_step(
        &mut self,
        name: &str,
        kind: StepKind,
        config: StepConfig,
    ) -> Result<StepOutput, EngineError> {
        let position = self.position;
        self.position += 1;

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

        // Record dependencies on previous step(s).
        self.record_dependencies_for(step.id).await?;

        // Transition to Running.
        let now = Utc::now();
        self.store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    started_at: Some(now),
                    ..StepUpdate::default()
                },
            )
            .await?;

        // Execute the operation.
        match execute_step_config(&config, &self.provider).await {
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
                            input_tokens: output.input_tokens,
                            output_tokens: output.output_tokens,
                            completed_at: Some(completed_at),
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

                // Update last_step_ids for downstream dependency tracking.
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
                    tracing::error!(step_id = %step.id, error = %store_err, "failed to persist step failure");
                }

                Err(err)
            }
        }
    }

    /// Record dependency edges from `step_id` to all `last_step_ids`.
    async fn record_dependencies_for(&self, step_id: Uuid) -> Result<(), EngineError> {
        if self.last_step_ids.is_empty() {
            return Ok(());
        }

        let deps: Vec<NewStepDependency> = self
            .last_step_ids
            .iter()
            .map(|&depends_on| NewStepDependency {
                step_id,
                depends_on,
            })
            .collect();

        self.store.create_step_dependencies(deps).await?;
        Ok(())
    }

    /// Access the store directly (advanced usage).
    pub fn store(&self) -> &Arc<dyn RunStore> {
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

impl std::fmt::Debug for WorkflowContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowContext")
            .field("run_id", &self.run_id)
            .field("position", &self.position)
            .field("total_cost_usd", &self.total_cost_usd)
            .finish_non_exhaustive()
    }
}
