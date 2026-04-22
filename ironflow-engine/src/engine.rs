//! The core [`Engine`] -- orchestrates workflow execution and persistence.
//!
//! The engine ties together a [`RunStore`] for persistence, an [`AgentProvider`]
//! for AI operations, and a registry of [`WorkflowHandler`]s.
//!
//! Handlers are Rust-native: steps can reference previous outputs, use native
//! `if`/`else`/`match` for conditional branching, and execute in parallel.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::{error, info};
use uuid::Uuid;

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::{RUN_COST_USD, RUN_DURATION_SECONDS, RUNS_ACTIVE, RUNS_TOTAL};
use ironflow_core::provider::AgentProvider;
use ironflow_store::error::StoreError;
use ironflow_store::models::{NewRun, Run, RunStatus, RunUpdate, TriggerKind};
use ironflow_store::store::Store;
#[cfg(feature = "prometheus")]
use metrics::{counter, gauge, histogram};

use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::handler::{WorkflowHandler, WorkflowInfo};
use crate::notify::{Event, EventPublisher, EventSubscriber};

/// The workflow orchestration engine.
///
/// Holds references to the store, agent provider, and a registry of
/// [`WorkflowHandler`]s.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_engine::engine::Engine;
/// use ironflow_engine::config::ShellConfig;
/// use ironflow_engine::handler::{WorkflowHandler, HandlerFuture, WorkflowInfo};
/// use ironflow_engine::context::WorkflowContext;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::models::TriggerKind;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use serde_json::json;
///
/// struct CiWorkflow;
/// impl WorkflowHandler for CiWorkflow {
///     fn name(&self) -> &str { "ci" }
///     fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
///         Box::pin(async move {
///             ctx.shell("test", ShellConfig::new("cargo test")).await?;
///             Ok(())
///         })
///     }
/// }
///
/// # async fn example() -> Result<(), ironflow_engine::error::EngineError> {
/// let store = Arc::new(InMemoryStore::new());
/// let provider = Arc::new(ClaudeCodeProvider::new());
/// let mut engine = Engine::new(store, provider);
/// engine.register(CiWorkflow)?;
///
/// let run = engine.run_handler("ci", TriggerKind::Manual, json!({})).await?;
/// println!("Run {} completed with status {:?}", run.id, run.status);
/// # Ok(())
/// # }
/// ```
pub struct Engine {
    store: Arc<dyn Store>,
    provider: Arc<dyn AgentProvider>,
    handlers: HashMap<String, Arc<dyn WorkflowHandler>>,
    event_publisher: EventPublisher,
}

/// Validate a workflow category path.
///
/// A category is a `/`-separated list of non-empty segments. This function
/// rejects empty paths, leading or trailing `/`, consecutive `/`, and
/// segments containing only whitespace.
///
/// # Errors
///
/// Returns [`EngineError::InvalidWorkflow`] when the category is malformed.
fn validate_category(handler_name: &str, category: &str) -> Result<(), EngineError> {
    let reject = |reason: &str| {
        Err(EngineError::InvalidWorkflow(format!(
            "handler '{handler_name}' has invalid category '{category}': {reason}"
        )))
    };

    if category.is_empty() {
        return reject("empty category");
    }
    if category.starts_with('/') {
        return reject("leading '/'");
    }
    if category.ends_with('/') {
        return reject("trailing '/'");
    }
    for segment in category.split('/') {
        if segment.is_empty() {
            return reject("empty segment (double '/')");
        }
        if segment.trim().is_empty() {
            return reject("whitespace-only segment");
        }
    }
    Ok(())
}

impl Engine {
    /// Create a new engine with the given store and agent provider.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    ///
    /// let engine = Engine::new(
    ///     Arc::new(InMemoryStore::new()),
    ///     Arc::new(ClaudeCodeProvider::new()),
    /// );
    /// ```
    pub fn new(store: Arc<dyn Store>, provider: Arc<dyn AgentProvider>) -> Self {
        Self {
            store,
            provider,
            handlers: HashMap::new(),
            event_publisher: EventPublisher::new(),
        }
    }

    /// Returns a reference to the backing store.
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Returns a reference to the agent provider.
    pub fn provider(&self) -> &Arc<dyn AgentProvider> {
        &self.provider
    }

    /// Build a [`WorkflowContext`] with access to the handler registry.
    fn build_context(&self, run_id: Uuid) -> WorkflowContext {
        let handlers = self.handlers.clone();
        let resolver: crate::context::HandlerResolver =
            Arc::new(move |name: &str| handlers.get(name).cloned());
        WorkflowContext::with_handler_resolver(
            run_id,
            self.store.clone(),
            self.provider.clone(),
            resolver,
        )
    }

    // -----------------------------------------------------------------------
    // Handler registration
    // -----------------------------------------------------------------------

    /// Register a [`WorkflowHandler`] for dynamic workflow execution.
    ///
    /// The handler is looked up by [`WorkflowHandler::name`] when executing
    /// or enqueuing.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if a handler with the same
    /// name is already registered.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::handler::{WorkflowHandler, HandlerFuture};
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    ///
    /// struct MyWorkflow;
    /// impl WorkflowHandler for MyWorkflow {
    ///     fn name(&self) -> &str { "my-workflow" }
    ///     fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move {
    ///             ctx.shell("step1", ShellConfig::new("echo done")).await?;
    ///             Ok(())
    ///         })
    ///     }
    /// }
    ///
    /// let mut engine = Engine::new(
    ///     Arc::new(InMemoryStore::new()),
    ///     Arc::new(ClaudeCodeProvider::new()),
    /// );
    /// engine.register(MyWorkflow)?;
    /// # Ok::<(), ironflow_engine::error::EngineError>(())
    /// ```
    pub fn register(&mut self, handler: impl WorkflowHandler + 'static) -> Result<(), EngineError> {
        let name = handler.name().to_string();
        if self.handlers.contains_key(&name) {
            return Err(EngineError::InvalidWorkflow(format!(
                "handler '{}' already registered",
                name
            )));
        }
        if let Some(category) = handler.category() {
            validate_category(&name, category)?;
        }
        self.handlers.insert(name, Arc::new(handler));
        Ok(())
    }

    /// Register a pre-boxed workflow handler.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if a handler with the same
    /// name is already registered or if its category is invalid.
    pub fn register_boxed(&mut self, handler: Box<dyn WorkflowHandler>) -> Result<(), EngineError> {
        let name = handler.name().to_string();
        if self.handlers.contains_key(&name) {
            return Err(EngineError::InvalidWorkflow(format!(
                "handler '{}' already registered",
                name
            )));
        }
        if let Some(category) = handler.category() {
            validate_category(&name, category)?;
        }
        self.handlers.insert(name, Arc::from(handler));
        Ok(())
    }

    /// Get a registered handler by name.
    pub fn get_handler(&self, name: &str) -> Option<&Arc<dyn WorkflowHandler>> {
        self.handlers.get(name)
    }

    /// List registered handler names.
    pub fn handler_names(&self) -> Vec<&str> {
        self.handlers.keys().map(|s| s.as_str()).collect()
    }

    /// Get detailed info about a registered workflow handler.
    pub fn handler_info(&self, name: &str) -> Option<WorkflowInfo> {
        self.handlers.get(name).map(|h| h.describe())
    }

    /// Register an event subscriber for domain events.
    ///
    /// The subscriber is called only for events whose type is in
    /// `event_types`. Pass [`Event::ALL`] to receive every event.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::notify::{Event, WebhookSubscriber};
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use std::sync::Arc;
    ///
    /// let mut engine = Engine::new(
    ///     Arc::new(InMemoryStore::new()),
    ///     Arc::new(ClaudeCodeProvider::new()),
    /// );
    ///
    /// engine.subscribe(
    ///     WebhookSubscriber::new("https://hooks.example.com/events"),
    ///     &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
    /// );
    /// ```
    pub fn subscribe(
        &mut self,
        subscriber: impl EventSubscriber + 'static,
        event_types: &[&'static str],
    ) {
        self.event_publisher.subscribe(subscriber, event_types);
    }

    /// Returns a reference to the event publisher.
    ///
    /// Useful for publishing events from outside the engine (e.g. auth
    /// routes in the API layer).
    pub fn event_publisher(&self) -> &EventPublisher {
        &self.event_publisher
    }

    // -----------------------------------------------------------------------
    // Dynamic workflow execution (WorkflowHandler)
    // -----------------------------------------------------------------------

    /// Execute a registered handler inline.
    ///
    /// Creates a run, builds a [`WorkflowContext`], calls the handler's
    /// [`execute`](WorkflowHandler::execute), and finalizes the run.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if no handler is registered
    /// with that name. Returns [`EngineError`] if execution fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::models::TriggerKind;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use serde_json::json;
    ///
    /// # async fn example(engine: &Engine) -> Result<(), ironflow_engine::error::EngineError> {
    /// let run = engine.run_handler("deploy", TriggerKind::Manual, json!({})).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[tracing::instrument(name = "engine.run_handler", skip_all, fields(workflow = %handler_name))]
    pub async fn run_handler(
        &self,
        handler_name: &str,
        trigger: TriggerKind,
        payload: Value,
    ) -> Result<Run, EngineError> {
        let handler = self
            .handlers
            .get(handler_name)
            .ok_or_else(|| {
                EngineError::InvalidWorkflow(format!("no handler registered: {handler_name}"))
            })?
            .clone();

        let handler_version = handler.version().map(str::to_string);
        let run = self
            .store
            .create_run(NewRun {
                workflow_name: handler_name.to_string(),
                trigger,
                payload,
                max_retries: 0,
                handler_version,
                labels: handler.default_labels(),
                scheduled_at: None,
            })
            .await?;

        let run_id = run.id;
        info!(run_id = %run_id, handler_version = run.handler_version.as_deref().unwrap_or(""), "run created");

        self.store
            .update_run_status(run_id, RunStatus::Running)
            .await?;

        #[cfg(feature = "prometheus")]
        gauge!(RUNS_ACTIVE, "workflow" => handler_name.to_string()).increment(1.0);

        let run_start = Instant::now();
        let mut ctx = self.build_context(run_id);

        let result = handler.execute(&mut ctx).await;
        self.finalize_run(run_id, handler_name, result, &ctx, run_start)
            .await
    }

    /// Enqueue a handler-based workflow for worker execution.
    ///
    /// The workflow name is stored in the run. The worker looks up the
    /// handler by name when executing.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if no handler is registered.
    #[tracing::instrument(name = "engine.enqueue_handler", skip_all, fields(workflow = %handler_name))]
    pub async fn enqueue_handler(
        &self,
        handler_name: &str,
        trigger: TriggerKind,
        payload: Value,
        max_retries: u32,
    ) -> Result<Run, EngineError> {
        self.enqueue_handler_with_options(
            handler_name,
            trigger,
            payload,
            max_retries,
            HashMap::new(),
            None,
        )
        .await
    }

    /// Enqueue a handler-based workflow with labels and optional deferred scheduling.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if no handler is registered.
    #[tracing::instrument(name = "engine.enqueue_handler_with_options", skip_all, fields(workflow = %handler_name))]
    pub async fn enqueue_handler_with_options(
        &self,
        handler_name: &str,
        trigger: TriggerKind,
        payload: Value,
        max_retries: u32,
        labels: HashMap<String, String>,
        scheduled_at: Option<DateTime<Utc>>,
    ) -> Result<Run, EngineError> {
        let handler = self.handlers.get(handler_name).ok_or_else(|| {
            EngineError::InvalidWorkflow(format!("no handler registered: {handler_name}"))
        })?;

        let handler_version = handler.version().map(str::to_string);
        let mut merged_labels = handler.default_labels();
        merged_labels.extend(labels);

        let run = self
            .store
            .create_run(NewRun {
                workflow_name: handler_name.to_string(),
                trigger,
                payload,
                max_retries,
                handler_version,
                labels: merged_labels,
                scheduled_at,
            })
            .await?;

        info!(run_id = %run.id, workflow = %handler_name, "handler run enqueued");
        Ok(run)
    }

    /// Execute a handler-based run (used by the worker after pick_next_pending).
    ///
    /// Looks up the handler by the run's `workflow_name` and executes it
    /// with a fresh [`WorkflowContext`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if no handler matches.
    #[tracing::instrument(name = "engine.execute_handler_run", skip_all, fields(run_id = %run_id))]
    pub async fn execute_handler_run(&self, run_id: Uuid) -> Result<Run, EngineError> {
        let run = self
            .store
            .get_run(run_id)
            .await?
            .ok_or(EngineError::Store(StoreError::RunNotFound(run_id)))?;

        let handler = self
            .handlers
            .get(&run.workflow_name)
            .ok_or_else(|| {
                EngineError::InvalidWorkflow(format!(
                    "no handler registered: {}",
                    run.workflow_name
                ))
            })?
            .clone();

        #[cfg(feature = "prometheus")]
        gauge!(RUNS_ACTIVE, "workflow" => run.workflow_name.clone()).increment(1.0);

        let run_start = Instant::now();
        let mut ctx = self.build_context(run_id);

        let result = handler.execute(&mut ctx).await;
        self.finalize_run(run_id, &run.workflow_name, result, &ctx, run_start)
            .await
    }

    /// Execute a run by its ID (used by the worker after pick_next_pending).
    ///
    /// Delegates to [`execute_handler_run`](Self::execute_handler_run).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the run is not found or execution fails.
    #[tracing::instrument(name = "engine.execute_run", skip_all, fields(run_id = %run_id))]
    pub async fn execute_run(&self, run_id: Uuid) -> Result<Run, EngineError> {
        self.execute_handler_run(run_id).await
    }

    /// Resume a run after human approval.
    ///
    /// Re-executes the handler with step replay: completed steps return
    /// cached output, approved approval steps are skipped, and execution
    /// continues from the first unexecuted step.
    ///
    /// Supports multiple approval gates -- each resume replays all prior
    /// steps and stops at the next approval (or completes the run).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if no handler matches.
    /// Returns [`EngineError`] if execution fails or hits another approval.
    #[tracing::instrument(name = "engine.resume_run", skip_all, fields(run_id = %run_id))]
    pub async fn resume_run(&self, run_id: Uuid) -> Result<Run, EngineError> {
        let run = self
            .store
            .get_run(run_id)
            .await?
            .ok_or(EngineError::Store(StoreError::RunNotFound(run_id)))?;

        let handler = self
            .handlers
            .get(&run.workflow_name)
            .ok_or_else(|| {
                EngineError::InvalidWorkflow(format!(
                    "no handler registered: {}",
                    run.workflow_name
                ))
            })?
            .clone();

        info!(run_id = %run_id, workflow = %run.workflow_name, "resuming run after approval");

        let run_start = Instant::now();
        let mut ctx = self.build_context(run_id);
        ctx.load_replay_steps().await?;

        let result = handler.execute(&mut ctx).await;
        self.finalize_run(run_id, &run.workflow_name, result, &ctx, run_start)
            .await
    }

    /// Finalize a run with the given result and context.
    ///
    /// On success: updates run to Completed with cost, duration, and completed_at.
    /// On failure: updates run to Failed with error, cost, duration, and completed_at.
    /// Always: fetches and returns the final Run.
    async fn finalize_run(
        &self,
        run_id: Uuid,
        workflow_name: &str,
        result: Result<(), EngineError>,
        ctx: &WorkflowContext,
        run_start: Instant,
    ) -> Result<Run, EngineError> {
        let total_duration = run_start.elapsed().as_millis() as u64;
        let completed_at = Utc::now();

        let final_status;
        let final_run;

        match result {
            Ok(()) => {
                final_status = RunStatus::Completed;
                final_run = self
                    .store
                    .update_run_returning(
                        run_id,
                        RunUpdate {
                            status: Some(RunStatus::Completed),
                            cost_usd: Some(ctx.total_cost_usd()),
                            duration_ms: Some(total_duration),
                            completed_at: Some(completed_at),
                            ..RunUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %run_id,
                    cost_usd = %ctx.total_cost_usd(),
                    duration_ms = total_duration,
                    "run completed"
                );
            }
            Err(EngineError::ApprovalRequired {
                run_id: approval_run_id,
                step_id,
                ref message,
            }) => {
                final_status = RunStatus::AwaitingApproval;
                final_run = self
                    .store
                    .update_run_returning(
                        run_id,
                        RunUpdate {
                            status: Some(RunStatus::AwaitingApproval),
                            cost_usd: Some(ctx.total_cost_usd()),
                            duration_ms: Some(total_duration),
                            ..RunUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %approval_run_id,
                    step_id = %step_id,
                    message = %message,
                    "run awaiting approval"
                );
            }
            Err(err) => {
                final_status = RunStatus::Failed;
                if let Err(store_err) = self
                    .store
                    .update_run(
                        run_id,
                        RunUpdate {
                            status: Some(RunStatus::Failed),
                            error: Some(err.to_string()),
                            cost_usd: Some(ctx.total_cost_usd()),
                            duration_ms: Some(total_duration),
                            completed_at: Some(completed_at),
                            ..RunUpdate::default()
                        },
                    )
                    .await
                {
                    error!(run_id = %run_id, store_error = %store_err, "failed to persist run failure");
                }

                error!(run_id = %run_id, error = %err, "run failed");

                self.publish_run_status_changed(
                    workflow_name,
                    run_id,
                    final_status,
                    Some(err.to_string()),
                    ctx,
                    total_duration,
                );

                #[cfg(feature = "prometheus")]
                self.emit_run_metrics(workflow_name, final_status, total_duration, ctx);

                return Err(err);
            }
        }

        self.publish_run_status_changed(
            workflow_name,
            run_id,
            final_status,
            None,
            ctx,
            total_duration,
        );

        #[cfg(feature = "prometheus")]
        self.emit_run_metrics(workflow_name, final_status, total_duration, ctx);

        Ok(final_run)
    }

    /// Emit Prometheus metrics for a completed run.
    #[cfg(feature = "prometheus")]
    fn emit_run_metrics(
        &self,
        workflow_name: &str,
        status: RunStatus,
        duration_ms: u64,
        ctx: &WorkflowContext,
    ) {
        let status_str = status.to_string();
        let wf = workflow_name.to_string();

        counter!(RUNS_TOTAL, "workflow" => wf.clone(), "status" => status_str.clone()).increment(1);
        histogram!(RUN_DURATION_SECONDS, "workflow" => wf.clone(), "status" => status_str)
            .record(duration_ms as f64 / 1000.0);
        histogram!(RUN_COST_USD, "workflow" => wf.clone()).record(
            ctx.total_cost_usd()
                .to_string()
                .parse::<f64>()
                .unwrap_or(0.0),
        );
        gauge!(RUNS_ACTIVE, "workflow" => wf).decrement(1.0);
    }

    /// Publish a run status changed event to all registered subscribers.
    ///
    /// `from` is always `Running` because `finalize_run` is only called
    /// from a running state.
    fn publish_run_status_changed(
        &self,
        workflow_name: &str,
        run_id: Uuid,
        to: RunStatus,
        error: Option<String>,
        ctx: &WorkflowContext,
        duration_ms: u64,
    ) {
        let now = Utc::now();
        let cost_usd = ctx.total_cost_usd();
        let wf = workflow_name.to_string();

        self.event_publisher.publish(Event::RunStatusChanged {
            run_id,
            workflow_name: wf.clone(),
            from: RunStatus::Running,
            to,
            error: error.clone(),
            cost_usd,
            duration_ms,
            at: now,
        });

        if to == RunStatus::Failed {
            self.event_publisher.publish(Event::RunFailed {
                run_id,
                workflow_name: wf,
                error,
                cost_usd,
                duration_ms,
                at: now,
            });
        }
    }
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("handlers", &self.handlers.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShellConfig;
    use crate::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_core::providers::record_replay::RecordReplayProvider;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::StepStatus;
    use serde_json::json;

    // Test handler that echoes a message via shell
    struct EchoWorkflow;

    impl WorkflowHandler for EchoWorkflow {
        fn name(&self) -> &str {
            "echo-workflow"
        }

        fn describe(&self) -> WorkflowInfo {
            WorkflowInfo {
                description: "A simple workflow that echoes hello".to_string(),
                source_code: None,
                sub_workflows: Vec::new(),
                category: None,
                version: self.version().map(str::to_string),
                input_schema: None,
                default_labels: HashMap::new(),
            }
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.shell("greet", ShellConfig::new("echo hello")).await?;
                Ok(())
            })
        }
    }

    // Test handler that fails
    struct FailingWorkflow;

    impl WorkflowHandler for FailingWorkflow {
        fn name(&self) -> &str {
            "failing-workflow"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.shell("fail", ShellConfig::new("exit 1")).await?;
                Ok(())
            })
        }
    }

    fn create_test_engine() -> Engine {
        let store = Arc::new(InMemoryStore::new());
        let inner = ClaudeCodeProvider::new();
        let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
            inner,
            "/tmp/ironflow-fixtures",
        ));
        Engine::new(store, provider)
    }

    #[test]
    fn engine_new_creates_instance() {
        let engine = create_test_engine();
        assert_eq!(engine.handler_names().len(), 0);
    }

    #[test]
    fn engine_register_handler() {
        let mut engine = create_test_engine();
        let result = engine.register(EchoWorkflow);
        assert!(result.is_ok());
        assert_eq!(engine.handler_names().len(), 1);
        assert!(engine.handler_names().contains(&"echo-workflow"));
    }

    #[test]
    fn engine_register_duplicate_returns_error() {
        let mut engine = create_test_engine();
        engine.register(EchoWorkflow).unwrap();
        let result = engine.register(EchoWorkflow);
        assert!(result.is_err());
    }

    #[test]
    fn engine_get_handler_found() {
        let mut engine = create_test_engine();
        engine.register(EchoWorkflow).unwrap();
        let handler = engine.get_handler("echo-workflow");
        assert!(handler.is_some());
    }

    #[test]
    fn engine_get_handler_not_found() {
        let engine = create_test_engine();
        let handler = engine.get_handler("nonexistent");
        assert!(handler.is_none());
    }

    #[test]
    fn engine_handler_names_lists_all() {
        let mut engine = create_test_engine();
        engine.register(EchoWorkflow).unwrap();
        engine.register(FailingWorkflow).unwrap();
        let names = engine.handler_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"echo-workflow"));
        assert!(names.contains(&"failing-workflow"));
    }

    #[test]
    fn engine_handler_info_returns_description() {
        let mut engine = create_test_engine();
        engine.register(EchoWorkflow).unwrap();
        let info = engine.handler_info("echo-workflow");
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.description, "A simple workflow that echoes hello");
    }

    struct CategorizedWorkflow;

    impl WorkflowHandler for CategorizedWorkflow {
        fn name(&self) -> &str {
            "categorized"
        }
        fn category(&self) -> Option<&str> {
            Some("data/etl")
        }
        fn execute<'a>(
            &'a self,
            _ctx: &'a mut WorkflowContext,
        ) -> crate::handler::HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    #[test]
    fn engine_default_describe_propagates_category() {
        let mut engine = create_test_engine();
        engine.register(CategorizedWorkflow).unwrap();
        let info = engine.handler_info("categorized").unwrap();
        assert_eq!(info.category.as_deref(), Some("data/etl"));
    }

    #[test]
    fn engine_default_describe_without_category() {
        let mut engine = create_test_engine();
        engine.register(EchoWorkflow).unwrap();
        let info = engine.handler_info("echo-workflow").unwrap();
        assert!(info.category.is_none());
    }

    struct BadCategoryWorkflow(&'static str);

    impl WorkflowHandler for BadCategoryWorkflow {
        fn name(&self) -> &str {
            "bad-category"
        }
        fn category(&self) -> Option<&str> {
            Some(self.0)
        }
        fn execute<'a>(
            &'a self,
            _ctx: &'a mut WorkflowContext,
        ) -> crate::handler::HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    #[test]
    fn engine_register_rejects_empty_category() {
        let mut engine = create_test_engine();
        let err = engine.register(BadCategoryWorkflow("")).unwrap_err();
        match err {
            EngineError::InvalidWorkflow(msg) => assert!(msg.contains("empty category")),
            other => panic!("expected InvalidWorkflow, got {other:?}"),
        }
    }

    #[test]
    fn engine_register_rejects_leading_slash_category() {
        let mut engine = create_test_engine();
        let err = engine
            .register(BadCategoryWorkflow("/data/etl"))
            .unwrap_err();
        match err {
            EngineError::InvalidWorkflow(msg) => assert!(msg.contains("leading '/'")),
            other => panic!("expected InvalidWorkflow, got {other:?}"),
        }
    }

    #[test]
    fn engine_register_rejects_trailing_slash_category() {
        let mut engine = create_test_engine();
        let err = engine
            .register(BadCategoryWorkflow("data/etl/"))
            .unwrap_err();
        match err {
            EngineError::InvalidWorkflow(msg) => assert!(msg.contains("trailing '/'")),
            other => panic!("expected InvalidWorkflow, got {other:?}"),
        }
    }

    #[test]
    fn engine_register_rejects_double_slash_category() {
        let mut engine = create_test_engine();
        let err = engine
            .register(BadCategoryWorkflow("data//etl"))
            .unwrap_err();
        match err {
            EngineError::InvalidWorkflow(msg) => assert!(msg.contains("empty segment")),
            other => panic!("expected InvalidWorkflow, got {other:?}"),
        }
    }

    #[test]
    fn engine_register_rejects_whitespace_only_segment_category() {
        let mut engine = create_test_engine();
        let err = engine
            .register(BadCategoryWorkflow("data/ /etl"))
            .unwrap_err();
        match err {
            EngineError::InvalidWorkflow(msg) => assert!(msg.contains("whitespace-only segment")),
            other => panic!("expected InvalidWorkflow, got {other:?}"),
        }
    }

    #[test]
    fn engine_register_accepts_valid_nested_category() {
        let mut engine = create_test_engine();
        assert!(engine.register(CategorizedWorkflow).is_ok());
    }

    #[tokio::test]
    async fn engine_unknown_workflow_returns_error() {
        let engine = create_test_engine();
        let result = engine
            .run_handler("unknown", TriggerKind::Manual, json!({}))
            .await;
        assert!(result.is_err());
        match result {
            Err(EngineError::InvalidWorkflow(msg)) => {
                assert!(msg.contains("no handler registered"));
            }
            _ => panic!("expected InvalidWorkflow error"),
        }
    }

    #[tokio::test]
    async fn engine_enqueue_handler_creates_pending_run() {
        let mut engine = create_test_engine();
        engine.register(EchoWorkflow).unwrap();

        let run = engine
            .enqueue_handler("echo-workflow", TriggerKind::Manual, json!({}), 0)
            .await
            .unwrap();
        assert_eq!(run.status.state, RunStatus::Pending);
        assert_eq!(run.workflow_name, "echo-workflow");
    }

    #[tokio::test]
    async fn engine_register_boxed() {
        let mut engine = create_test_engine();
        let handler: Box<dyn WorkflowHandler> = Box::new(EchoWorkflow);
        let result = engine.register_boxed(handler);
        assert!(result.is_ok());
        assert_eq!(engine.handler_names().len(), 1);
    }

    #[tokio::test]
    async fn engine_store_and_provider_accessors() {
        let store = Arc::new(InMemoryStore::new());
        let inner = ClaudeCodeProvider::new();
        let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
            inner,
            "/tmp/ironflow-fixtures",
        ));
        let engine = Engine::new(store.clone(), provider.clone());

        // Verify accessors return references
        let _ = engine.store();
        let _ = engine.provider();
    }

    // -----------------------------------------------------------------------
    // Operation trait tests
    // -----------------------------------------------------------------------

    use crate::operation::Operation;
    use ironflow_store::models::StepKind;
    use std::future::Future;
    use std::pin::Pin;

    struct FakeGitlabOp {
        project_id: u64,
        title: String,
    }

    impl Operation for FakeGitlabOp {
        fn kind(&self) -> &str {
            "gitlab"
        }

        fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
            Box::pin(async move {
                Ok(json!({
                    "issue_id": 42,
                    "project_id": self.project_id,
                    "title": self.title,
                }))
            })
        }

        fn input(&self) -> Option<Value> {
            Some(json!({
                "project_id": self.project_id,
                "title": self.title,
            }))
        }
    }

    struct FailingOp;

    impl Operation for FailingOp {
        fn kind(&self) -> &str {
            "broken-service"
        }

        fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
            Box::pin(async move { Err(EngineError::StepConfig("service unavailable".to_string())) })
        }
    }

    struct OperationWorkflow;

    impl WorkflowHandler for OperationWorkflow {
        fn name(&self) -> &str {
            "operation-workflow"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                let op = FakeGitlabOp {
                    project_id: 123,
                    title: "Bug report".to_string(),
                };
                ctx.operation("create-issue", &op).await?;
                Ok(())
            })
        }
    }

    struct FailingOperationWorkflow;

    impl WorkflowHandler for FailingOperationWorkflow {
        fn name(&self) -> &str {
            "failing-operation-workflow"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.operation("broken-call", &FailingOp).await?;
                Ok(())
            })
        }
    }

    struct MixedWorkflow;

    impl WorkflowHandler for MixedWorkflow {
        fn name(&self) -> &str {
            "mixed-workflow"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.shell("build", ShellConfig::new("echo built")).await?;
                let op = FakeGitlabOp {
                    project_id: 456,
                    title: "Deploy done".to_string(),
                };
                let result = ctx.operation("notify-gitlab", &op).await?;
                assert_eq!(result.output["issue_id"], 42);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn operation_step_happy_path() {
        let mut engine = create_test_engine();
        engine.register(OperationWorkflow).unwrap();

        let run = engine
            .run_handler("operation-workflow", TriggerKind::Manual, json!({}))
            .await
            .unwrap();

        assert_eq!(run.status.state, RunStatus::Completed);

        let steps = engine.store().list_steps(run.id).await.unwrap();

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].name, "create-issue");
        assert_eq!(steps[0].kind, StepKind::Custom("gitlab".to_string()));
        assert_eq!(
            steps[0].status.state,
            ironflow_store::models::StepStatus::Completed
        );

        let output = steps[0].output.as_ref().unwrap();
        assert_eq!(output["issue_id"], 42);
        assert_eq!(output["project_id"], 123);

        let input = steps[0].input.as_ref().unwrap();
        assert_eq!(input["project_id"], 123);
        assert_eq!(input["title"], "Bug report");
    }

    #[tokio::test]
    async fn operation_step_failure_marks_run_failed() {
        let mut engine = create_test_engine();
        engine.register(FailingOperationWorkflow).unwrap();

        let result = engine
            .run_handler("failing-operation-workflow", TriggerKind::Manual, json!({}))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn operation_mixed_with_shell_steps() {
        let mut engine = create_test_engine();
        engine.register(MixedWorkflow).unwrap();

        let run = engine
            .run_handler("mixed-workflow", TriggerKind::Manual, json!({}))
            .await
            .unwrap();

        assert_eq!(run.status.state, RunStatus::Completed);

        let steps = engine.store().list_steps(run.id).await.unwrap();

        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].kind, StepKind::Shell);
        assert_eq!(steps[1].kind, StepKind::Custom("gitlab".to_string()));
        assert_eq!(steps[0].position, 0);
        assert_eq!(steps[1].position, 1);
    }

    // -----------------------------------------------------------------------
    // Approval + resume tests
    // -----------------------------------------------------------------------

    use crate::config::ApprovalConfig;

    struct SingleApprovalWorkflow;

    impl WorkflowHandler for SingleApprovalWorkflow {
        fn name(&self) -> &str {
            "single-approval"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.shell("build", ShellConfig::new("echo built")).await?;
                ctx.approval("gate", ApprovalConfig::new("OK?")).await?;
                ctx.shell("deploy", ShellConfig::new("echo deployed"))
                    .await?;
                Ok(())
            })
        }
    }

    struct DoubleApprovalWorkflow;

    impl WorkflowHandler for DoubleApprovalWorkflow {
        fn name(&self) -> &str {
            "double-approval"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.shell("build", ShellConfig::new("echo built")).await?;
                ctx.approval("staging-gate", ApprovalConfig::new("Deploy staging?"))
                    .await?;
                ctx.shell("deploy-staging", ShellConfig::new("echo staging"))
                    .await?;
                ctx.approval("prod-gate", ApprovalConfig::new("Deploy prod?"))
                    .await?;
                ctx.shell("deploy-prod", ShellConfig::new("echo prod"))
                    .await?;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn approval_pauses_run() {
        let mut engine = create_test_engine();
        engine.register(SingleApprovalWorkflow).unwrap();

        let run = engine
            .run_handler("single-approval", TriggerKind::Manual, json!({}))
            .await
            .unwrap();

        assert_eq!(run.status.state, RunStatus::AwaitingApproval);

        let steps = engine.store().list_steps(run.id).await.unwrap();
        assert_eq!(steps.len(), 2); // build + approval gate
        assert_eq!(steps[0].kind, StepKind::Shell);
        assert_eq!(steps[0].status.state, StepStatus::Completed);
        assert_eq!(steps[1].kind, StepKind::Approval);
        assert_eq!(steps[1].status.state, StepStatus::AwaitingApproval);
    }

    #[tokio::test]
    async fn approval_resume_completes_run() {
        let mut engine = create_test_engine();
        engine.register(SingleApprovalWorkflow).unwrap();

        // First execution: pauses at approval
        let run = engine
            .run_handler("single-approval", TriggerKind::Manual, json!({}))
            .await
            .unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);

        // Simulate approval: transition to Running
        engine
            .store()
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        // Resume: replays build, skips approval, executes deploy
        let resumed = engine.resume_run(run.id).await.unwrap();
        assert_eq!(resumed.status.state, RunStatus::Completed);

        let steps = engine.store().list_steps(run.id).await.unwrap();
        assert_eq!(steps.len(), 3); // build + approval + deploy
        assert_eq!(steps[0].name, "build");
        assert_eq!(steps[0].status.state, StepStatus::Completed);
        assert_eq!(steps[1].name, "gate");
        assert_eq!(steps[1].kind, StepKind::Approval);
        assert_eq!(steps[1].status.state, StepStatus::Completed);
        assert_eq!(steps[2].name, "deploy");
        assert_eq!(steps[2].status.state, StepStatus::Completed);
    }

    #[tokio::test]
    async fn double_approval_two_resumes() {
        let mut engine = create_test_engine();
        engine.register(DoubleApprovalWorkflow).unwrap();

        // First execution: pauses at staging-gate
        let run = engine
            .run_handler("double-approval", TriggerKind::Manual, json!({}))
            .await
            .unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);

        let steps = engine.store().list_steps(run.id).await.unwrap();
        assert_eq!(steps.len(), 2); // build + staging-gate

        // First approval
        engine
            .store()
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        let resumed = engine.resume_run(run.id).await.unwrap();
        assert_eq!(resumed.status.state, RunStatus::AwaitingApproval);

        let steps = engine.store().list_steps(run.id).await.unwrap();
        assert_eq!(steps.len(), 4); // build + staging-gate + deploy-staging + prod-gate

        // Second approval
        engine
            .store()
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        let final_run = engine.resume_run(run.id).await.unwrap();
        assert_eq!(final_run.status.state, RunStatus::Completed);

        let steps = engine.store().list_steps(run.id).await.unwrap();
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[0].name, "build");
        assert_eq!(steps[1].name, "staging-gate");
        assert_eq!(steps[2].name, "deploy-staging");
        assert_eq!(steps[3].name, "prod-gate");
        assert_eq!(steps[4].name, "deploy-prod");

        for step in &steps {
            assert_eq!(step.status.state, StepStatus::Completed);
        }
    }
}
