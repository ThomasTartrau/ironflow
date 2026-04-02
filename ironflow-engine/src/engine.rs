//! The core [`Engine`] -- orchestrates workflow execution and persistence.
//!
//! The engine ties together a [`RunStore`] for persistence, an [`AgentProvider`]
//! for AI operations, and a registry of [`WorkflowHandler`]s.
//!
//! Handlers are Rust-native: steps can reference previous outputs, use native
//! `if`/`else`/`match` for conditional branching, and execute in parallel.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use serde_json::Value;
use tracing::{error, info};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_store::error::StoreError;
use ironflow_store::models::{NewRun, Run, RunStatus, RunUpdate, TriggerKind};
use ironflow_store::store::RunStore;

use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::handler::{WorkflowHandler, WorkflowInfo};

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
    store: Arc<dyn RunStore>,
    provider: Arc<dyn AgentProvider>,
    handlers: HashMap<String, Arc<dyn WorkflowHandler>>,
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
    pub fn new(store: Arc<dyn RunStore>, provider: Arc<dyn AgentProvider>) -> Self {
        Self {
            store,
            provider,
            handlers: HashMap::new(),
        }
    }

    /// Returns a reference to the backing store.
    pub fn store(&self) -> &Arc<dyn RunStore> {
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
        self.handlers.insert(name, Arc::new(handler));
        Ok(())
    }

    /// Register a pre-boxed workflow handler.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if a handler with the same
    /// name is already registered.
    pub fn register_boxed(&mut self, handler: Box<dyn WorkflowHandler>) -> Result<(), EngineError> {
        let name = handler.name().to_string();
        if self.handlers.contains_key(&name) {
            return Err(EngineError::InvalidWorkflow(format!(
                "handler '{}' already registered",
                name
            )));
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

        let run = self
            .store
            .create_run(NewRun {
                workflow_name: handler_name.to_string(),
                trigger,
                payload,
                max_retries: 0,
            })
            .await?;

        let run_id = run.id;
        info!(run_id = %run_id, "run created");

        self.store
            .update_run_status(run_id, RunStatus::Running)
            .await?;

        let run_start = Instant::now();
        let mut ctx = self.build_context(run_id);

        let result = handler.execute(&mut ctx).await;
        self.finalize_run(run_id, result, &ctx, run_start).await
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
        if !self.handlers.contains_key(handler_name) {
            return Err(EngineError::InvalidWorkflow(format!(
                "no handler registered: {handler_name}"
            )));
        }

        let run = self
            .store
            .create_run(NewRun {
                workflow_name: handler_name.to_string(),
                trigger,
                payload,
                max_retries,
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

        let run_start = Instant::now();
        let mut ctx = self.build_context(run_id);

        let result = handler.execute(&mut ctx).await;
        self.finalize_run(run_id, result, &ctx, run_start).await
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

    /// Finalize a run with the given result and context.
    ///
    /// On success: updates run to Completed with cost, duration, and completed_at.
    /// On failure: updates run to Failed with error, cost, duration, and completed_at.
    /// Always: fetches and returns the final Run.
    ///
    /// TODO: `get_run` at the end could be optimized by using an `update_run_returning`
    /// method if the store supports it.
    async fn finalize_run(
        &self,
        run_id: Uuid,
        result: Result<(), EngineError>,
        ctx: &WorkflowContext,
        run_start: Instant,
    ) -> Result<Run, EngineError> {
        let total_duration = run_start.elapsed().as_millis() as u64;
        let completed_at = Utc::now();

        match result {
            Ok(()) => {
                self.store
                    .update_run(
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
            Err(err) => {
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
                return Err(err);
            }
        }

        self.store
            .get_run(run_id)
            .await?
            .ok_or(EngineError::Store(StoreError::RunNotFound(run_id)))
    }
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
}
