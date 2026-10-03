//! [`TestEngine`] -- the builder that runs a handler against mocked steps.

use std::fmt;
use std::sync::Arc;

#[cfg(feature = "secret-store")]
use rand::random;
use serde_json::Value;
use uuid::Uuid;

use ironflow_core::decision::DecisionProvider;
use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider};
use ironflow_core::providers::record_replay::RecordReplayProvider;
#[cfg(feature = "secret-store")]
use ironflow_store::crypto::MasterKey;
use ironflow_store::error::StoreError;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, TriggerKind};
use ironflow_store::store::{RunStore, Store};
#[cfg(feature = "secret-store")]
use ironflow_store::workflow_secrets::ScopedSecretStore;

use crate::config::{HttpConfig, HumanInputConfig, ShellConfig};
use crate::engine::{Engine, WorkflowResult, chain_root};
use crate::error::EngineError;
use crate::executor::{ApprovalOutcome, HumanInputOutcome, SignalOutcome, StepInterceptor};
use crate::handler::WorkflowHandler;
use crate::testing::mocks::{
    MissingAgentProvider, MockAgentProvider, MockHttpResponse, MockInterceptor, MockShellOutput,
};
use crate::testing::result::TestResult;

/// Message of the assert guarding every builder method.
const CONFIGURE_BEFORE_RUN: &str = "configure the TestEngine before its first run";

/// Runs a [`WorkflowHandler`] against an in-memory store with mocked steps.
///
/// See the [module documentation](crate::testing) for what the harness replaces
/// and what it does not.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::prelude::*;
/// use ironflow_engine::testing::{MockShellOutput, TestEngine};
/// use ironflow_store::models::RunStatus;
/// use serde_json::json;
///
/// # struct Deploy;
/// # impl WorkflowHandler for Deploy {
/// #     fn name(&self) -> &str { "deploy" }
/// #     fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
/// #         Box::pin(async move { ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?; Ok(()) })
/// #     }
/// # }
/// # async fn example() -> Result<(), EngineError> {
/// let result = TestEngine::new()
///     .with_handler(Deploy)
///     .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("deployed")))
///     .run(json!({"env": "prod"}))
///     .await?;
///
/// assert_eq!(result.status(), RunStatus::Completed);
/// # Ok(())
/// # }
/// ```
pub struct TestEngine {
    store: Arc<InMemoryStore>,
    handlers: Vec<Box<dyn WorkflowHandler>>,
    primary: Option<String>,
    provider: Option<Arc<dyn AgentProvider>>,
    decision_provider: Option<Arc<dyn DecisionProvider>>,
    mocks: MockInterceptor,
    engine: Option<Engine>,
    #[cfg(feature = "secret-store")]
    secrets: Vec<(String, String)>,
}

impl Default for TestEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TestEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestEngine")
            .field("primary", &self.primary)
            .field("mocks", &self.mocks)
            .field("built", &self.engine.is_some())
            .finish_non_exhaustive()
    }
}

impl TestEngine {
    /// A harness with no handler and no mock.
    ///
    /// With the `secret-store` feature the store gets a freshly generated
    /// master key, so reading a secret that was never set resolves to `None`
    /// instead of failing with "no master key configured". Seed values with
    /// `with_secret`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::testing::TestEngine;
    ///
    /// let harness = TestEngine::new();
    /// assert!(format!("{harness:?}").contains("TestEngine"));
    /// ```
    pub fn new() -> Self {
        #[cfg_attr(not(feature = "secret-store"), allow(unused_mut))]
        let mut store = InMemoryStore::new();
        #[cfg(feature = "secret-store")]
        store.set_master_key(
            MasterKey::from_bytes(&random::<[u8; 32]>())
                .expect("32 bytes is a valid master key length"),
        );
        Self {
            store: Arc::new(store),
            handlers: Vec::new(),
            primary: None,
            provider: None,
            decision_provider: None,
            mocks: MockInterceptor::new(),
            engine: None,
            #[cfg(feature = "secret-store")]
            secrets: Vec::new(),
        }
    }

    /// Register a handler. The first one registered is what
    /// [`run`](Self::run) executes.
    ///
    /// # Panics
    ///
    /// Panics when called after the first run: the engine is built once, so a
    /// later registration would be silently ignored.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::prelude::*;
    /// use ironflow_engine::testing::TestEngine;
    ///
    /// # struct Deploy;
    /// # impl WorkflowHandler for Deploy {
    /// #     fn name(&self) -> &str { "deploy" }
    /// #     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    /// #         Box::pin(async move { Ok(()) })
    /// #     }
    /// # }
    /// let harness = TestEngine::new().with_handler(Deploy);
    /// # let _ = harness;
    /// ```
    pub fn with_handler(mut self, handler: impl WorkflowHandler + 'static) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        if self.primary.is_none() {
            self.primary = Some(handler.name().to_string());
        }
        self.handlers.push(Box::new(handler));
        self
    }

    /// Answer every shell step with `f` instead of spawning a process.
    ///
    /// Returning `Err` reproduces a shell failure the same way a non-zero
    /// [`MockShellOutput::exit_code`] does: a non-zero exit code is an error
    /// unless the step set `exit_code_as_output`.
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::testing::{MockShellOutput, TestEngine};
    ///
    /// let harness = TestEngine::new().with_mock_shell(|cfg| {
    ///     if cfg.command.starts_with("git ") {
    ///         Ok(MockShellOutput::ok("abc1234"))
    ///     } else {
    ///         Ok(MockShellOutput::failed(127, "command not found"))
    ///     }
    /// });
    /// # let _ = harness;
    /// ```
    pub fn with_mock_shell(
        mut self,
        f: impl Fn(&ShellConfig) -> Result<MockShellOutput, OperationError> + Send + Sync + 'static,
    ) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.mocks = self.mocks.shell(f);
        self
    }

    /// Answer every HTTP step with `f` instead of sending a request.
    ///
    /// A non-2xx [`MockHttpResponse`] is a normal output, like in production.
    /// Return `Err(OperationError::Http { status: None, .. })` to simulate a
    /// transport failure.
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::testing::{MockHttpResponse, TestEngine};
    /// use serde_json::json;
    ///
    /// let harness = TestEngine::new()
    ///     .with_mock_http(|_cfg| Ok(MockHttpResponse::json(201, &json!({"id": 7}))));
    /// # let _ = harness;
    /// ```
    pub fn with_mock_http(
        mut self,
        f: impl Fn(&HttpConfig) -> Result<MockHttpResponse, OperationError> + Send + Sync + 'static,
    ) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.mocks = self.mocks.http(f);
        self
    }

    /// Resolve every approval gate with `outcome` instead of suspending.
    ///
    /// Without this, a gated handler ends the run in
    /// [`RunStatus::AwaitingApproval`] and [`resume`](Self::resume) continues it.
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::testing::{ApprovalOutcome, TestEngine};
    ///
    /// let harness = TestEngine::new().with_mock_approval(ApprovalOutcome::Approved);
    /// # let _ = harness;
    /// ```
    pub fn with_mock_approval(mut self, outcome: ApprovalOutcome) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.mocks = self.mocks.approval(outcome);
        self
    }

    /// Answer every human input step with `f` instead of suspending.
    ///
    /// `f` receives the step name and its config. Without this, a handler that
    /// asks for a human input ends the run in [`RunStatus::AwaitingApproval`];
    /// write the answer on the step through the store, then call
    /// [`resume`](Self::resume).
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::testing::{HumanInputOutcome, TestEngine};
    /// use serde_json::json;
    ///
    /// let harness = TestEngine::new().with_mock_human_input(|_name, _cfg| {
    ///     HumanInputOutcome::Provided(json!({"answers": ["staging"]}))
    /// });
    /// # let _ = harness;
    /// ```
    pub fn with_mock_human_input(
        mut self,
        f: impl Fn(&str, &HumanInputConfig) -> HumanInputOutcome + Send + Sync + 'static,
    ) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.mocks = self.mocks.human_input(f);
        self
    }

    /// Resolve every signal step with `f` instead of waiting for a signal.
    ///
    /// `f` receives the step name, the signal name and the key, and returns
    /// [`SignalOutcome::Received`] with the payload, or
    /// [`SignalOutcome::TimedOut`] to make `ctx.wait_for_signal` return `None`.
    /// Without this, a handler that waits for a signal ends the run in
    /// [`RunStatus::Sleeping`].
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::testing::{SignalOutcome, TestEngine};
    /// use serde_json::json;
    ///
    /// let harness = TestEngine::new().with_mock_signal(|_step, _name, _key| {
    ///     SignalOutcome::Received(json!({"status": "success"}))
    /// });
    /// # let _ = harness;
    /// ```
    pub fn with_mock_signal(
        mut self,
        f: impl Fn(&str, &str, &str) -> SignalOutcome + Send + Sync + 'static,
    ) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.mocks = self.mocks.signal(f);
        self
    }

    /// Answer every agent step with `f` instead of invoking a backend.
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::provider::AgentOutput;
    /// use ironflow_engine::testing::TestEngine;
    /// use serde_json::json;
    ///
    /// let harness = TestEngine::new()
    ///     .with_mock_agent(|_cfg| Ok(AgentOutput::new(json!({"score": 9}))));
    /// # let _ = harness;
    /// ```
    pub fn with_mock_agent(
        mut self,
        f: impl Fn(&AgentConfig) -> Result<AgentOutput, AgentError> + Send + Sync + 'static,
    ) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.provider = Some(Arc::new(MockAgentProvider::new(f)));
        self
    }

    /// Replay agent steps from fixtures recorded in `fixtures_dir`.
    ///
    /// `fixtures_dir` is the **directory**, not a file:
    /// [`RecordReplayProvider`] keys each fixture by a hash of the
    /// [`AgentConfig`] and stores it as `<hash>.json` inside it. A missing
    /// fixture falls back to [`MissingAgentProvider`], so the step fails loudly
    /// instead of reaching the real Claude CLI.
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::testing::TestEngine;
    ///
    /// let harness = TestEngine::new().with_recorded_agent("tests/fixtures");
    /// # let _ = harness;
    /// ```
    pub fn with_recorded_agent(mut self, fixtures_dir: &str) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.provider = Some(Arc::new(RecordReplayProvider::replay(
            MissingAgentProvider,
            fixtures_dir,
        )));
        self
    }

    /// Use an arbitrary [`AgentProvider`] for agent steps.
    ///
    /// The escape hatch for anything the three `with_mock_*` methods do not
    /// cover, such as recording new fixtures.
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use ironflow_core::provider::AgentProvider;
    /// use ironflow_engine::testing::TestEngine;
    ///
    /// # fn example(provider: Arc<dyn AgentProvider>) {
    /// let harness = TestEngine::new().with_agent_provider(provider);
    /// # let _ = harness;
    /// # }
    /// ```
    pub fn with_agent_provider(mut self, provider: Arc<dyn AgentProvider>) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.provider = Some(provider);
        self
    }

    /// Use a [`DecisionProvider`] for `ctx.decision(...)` steps.
    ///
    /// Decision steps are not intercepted: without a provider they fail with
    /// [`EngineError::NoDecisionProvider`].
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use ironflow_core::decision::DecisionProvider;
    /// use ironflow_engine::testing::TestEngine;
    ///
    /// # fn example(provider: Arc<dyn DecisionProvider>) {
    /// let harness = TestEngine::new().with_decision_provider(provider);
    /// # let _ = harness;
    /// # }
    /// ```
    pub fn with_decision_provider(mut self, provider: Arc<dyn DecisionProvider>) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.decision_provider = Some(provider);
        self
    }

    /// Seed a secret readable by the workflow under test.
    ///
    /// The secret is written in the scope of the workflow being run, the same
    /// scope as `ctx.secrets()` and as the operations run through
    /// `ctx.operation`. Setting the same key twice keeps the last value.
    /// Requires the `secret-store` feature.
    ///
    /// # Panics
    ///
    /// Panics when called after the first run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::testing::TestEngine;
    ///
    /// let harness = TestEngine::new().with_secret("git_token", "ghp_test");
    /// # let _ = harness;
    /// ```
    #[cfg(feature = "secret-store")]
    pub fn with_secret(mut self, key: &str, value: &str) -> Self {
        assert!(self.engine.is_none(), "{CONFIGURE_BEFORE_RUN}");
        self.secrets.push((key.to_string(), value.to_string()));
        self
    }

    /// The store backing this harness, for assertions the accessors do not
    /// cover (child runs, step dependencies, logs).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::testing::TestEngine;
    /// use ironflow_store::store::RunStore;
    /// use uuid::Uuid;
    ///
    /// # async fn example(harness: &TestEngine, run_id: Uuid) -> Result<(), EngineError> {
    /// let steps = harness.store().list_steps(run_id).await?;
    /// assert!(!steps.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn store(&self) -> Arc<InMemoryStore> {
        self.store.clone()
    }

    /// Build the underlying [`Engine`] on first use.
    ///
    /// Handlers are drained into it, which is why every builder method asserts
    /// that no run has happened yet.
    fn ensure_engine(&mut self) -> Result<(), EngineError> {
        if self.engine.is_some() {
            return Ok(());
        }

        let store: Arc<dyn Store> = self.store.clone();
        let provider = self
            .provider
            .clone()
            .unwrap_or_else(|| Arc::new(MissingAgentProvider));
        let mocks: Arc<dyn StepInterceptor> = Arc::new(self.mocks.clone());
        let mut engine = Engine::new(store, provider).with_step_interceptor(mocks);
        if let Some(decision_provider) = self.decision_provider.clone() {
            engine = engine.with_decision_provider(decision_provider);
        }
        for handler in self.handlers.drain(..) {
            engine.register_boxed(handler)?;
        }

        self.engine = Some(engine);
        Ok(())
    }

    /// Run the first handler registered with [`with_handler`](Self::with_handler).
    ///
    /// A handler that fails is not an error: the returned [`TestResult`] then
    /// carries [`RunStatus::Failed`] and the message in
    /// [`TestResult::error`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] when no handler was registered
    /// or two handlers share a name, and [`EngineError::Store`] when the
    /// in-memory store rejects a write.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::prelude::*;
    /// use ironflow_engine::testing::{MockShellOutput, TestEngine};
    /// use serde_json::json;
    ///
    /// # struct Deploy;
    /// # impl WorkflowHandler for Deploy {
    /// #     fn name(&self) -> &str { "deploy" }
    /// #     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    /// #         Box::pin(async move { Ok(()) })
    /// #     }
    /// # }
    /// # async fn example() -> Result<(), EngineError> {
    /// let result = TestEngine::new()
    ///     .with_handler(Deploy)
    ///     .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("done")))
    ///     .run(json!({}))
    ///     .await?;
    /// assert!(result.is_completed());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn run(&mut self, payload: Value) -> Result<TestResult, EngineError> {
        let name = self.primary.clone().ok_or_else(|| {
            EngineError::InvalidWorkflow(
                "TestEngine has no handler: call with_handler(...) first".to_string(),
            )
        })?;
        self.run_workflow(&name, payload).await
    }

    /// Run a specific registered handler by name.
    ///
    /// # Errors
    ///
    /// Same as [`run`](Self::run), plus [`EngineError::InvalidWorkflow`] when
    /// `name` matches no registered handler.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::testing::TestEngine;
    /// use serde_json::json;
    ///
    /// # async fn example(harness: &mut TestEngine) -> Result<(), EngineError> {
    /// let result = harness.run_workflow("child", json!({"id": 1})).await?;
    /// assert!(result.is_completed());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn run_workflow(
        &mut self,
        name: &str,
        payload: Value,
    ) -> Result<TestResult, EngineError> {
        self.ensure_engine()?;

        #[cfg(feature = "secret-store")]
        {
            let scoped = ScopedSecretStore::for_workflow(
                Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()),
                self.store.clone(),
            );
            for (key, value) in &self.secrets {
                scoped.set(key, value).await?;
            }
        }

        let engine = self.engine.as_ref().expect("ensure_engine built it");

        // Enqueue then execute, the way the worker does, so the run id is known
        // before execution and a failed run can still be read back.
        let trigger = TriggerKind::Manual;
        let run = engine.enqueue_handler(name, trigger, payload, 0).await?;
        self.store
            .update_run_status(run.id, RunStatus::Running)
            .await?;
        let execution = engine.execute_handler_run(run.id).await;
        self.collect(run.id, execution).await
    }

    /// Resume a run suspended on an approval gate, a human input, a delay or a
    /// signal wait, the way the API server and the waker do.
    ///
    /// A suspended sub-workflow child run is resumed through its root run,
    /// like in production: the root replays and re-enters the child. The
    /// returned [`TestResult`] then describes the root run.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] when the run does not exist or is not
    /// resumable, and [`EngineError::InvalidWorkflow`] when its handler is no
    /// longer registered.
    ///
    /// # Panics
    ///
    /// Never in practice: the engine is built by `ensure_engine` just before.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::testing::TestEngine;
    /// use ironflow_store::models::RunStatus;
    /// use serde_json::json;
    ///
    /// # async fn example(harness: &mut TestEngine) -> Result<(), EngineError> {
    /// let suspended = harness.run(json!({})).await?;
    /// assert_eq!(suspended.status(), RunStatus::AwaitingApproval);
    ///
    /// let resumed = harness.resume(suspended.run_id()).await?;
    /// assert_eq!(resumed.status(), RunStatus::Completed);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume(&mut self, run_id: Uuid) -> Result<TestResult, EngineError> {
        self.ensure_engine()?;
        let engine = self.engine.as_ref().expect("ensure_engine built it");

        let run = self
            .store
            .get_run(run_id)
            .await?
            .ok_or(EngineError::Store(StoreError::RunNotFound(run_id)))?;
        // A sleeping run is woken through `Pending`, like the waker does.
        if run.status.state == RunStatus::Sleeping {
            self.store
                .update_run_status(run_id, RunStatus::Pending)
                .await?;
        }
        self.store
            .update_run_status(run_id, RunStatus::Running)
            .await?;
        let execution = engine.resume_run(run_id).await;
        let reported = chain_root(&run).unwrap_or(run_id);
        self.collect(reported, execution).await
    }

    /// Read the run and its steps back from the store.
    ///
    /// The engine returns `Err` for a failed run, so the steps are always read
    /// from the store rather than from the execution result.
    async fn collect(
        &self,
        run_id: Uuid,
        execution: Result<WorkflowResult, EngineError>,
    ) -> Result<TestResult, EngineError> {
        let run = self
            .store
            .get_run(run_id)
            .await?
            .ok_or(EngineError::Store(StoreError::RunNotFound(run_id)))?;
        let steps = self.store.list_steps(run_id).await?;

        let (step_results, error) = match execution {
            Ok(result) => (result.steps, run.error.clone()),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };

        Ok(TestResult::new(run, steps, step_results, error))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::json;
    use tokio::time::timeout;

    use ironflow_core::operation::{Operation, OperationContext};

    use crate::context::WorkflowContext;
    use crate::handler::HandlerFuture;

    use super::*;

    /// Reads `git_token` through the operation context.
    struct ReadsToken;

    #[async_trait]
    impl Operation for ReadsToken {
        fn kind(&self) -> &str {
            "reads-token"
        }

        async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
            let token = ctx.secrets().get("git_token").await?;
            Ok(json!({"token": token.map(|secret| secret.value)}))
        }
    }

    /// Runs [`ReadsToken`] through `ctx.operation`.
    struct UsesSecretOp;

    impl WorkflowHandler for UsesSecretOp {
        fn name(&self) -> &str {
            "uses-secret-op"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.operation("read-token", &ReadsToken).await?;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn operation_reading_absent_secret_completes_under_test_engine() {
        timeout(Duration::from_secs(10), async {
            let result = TestEngine::new()
                .with_handler(UsesSecretOp)
                .run(json!({}))
                .await
                .unwrap();

            assert_eq!(
                result.status(),
                RunStatus::Completed,
                "{:?}",
                result.error()
            );
            assert!(result.step("read-token").output()["token"].is_null());
        })
        .await
        .expect("test timed out");
    }

    #[cfg(feature = "secret-store")]
    #[tokio::test]
    async fn with_secret_value_reaches_operation_via_ctx_operation() {
        timeout(Duration::from_secs(10), async {
            let result = TestEngine::new()
                .with_handler(UsesSecretOp)
                .with_secret("git_token", "ghp_test")
                .run(json!({}))
                .await
                .unwrap();

            assert_eq!(
                result.status(),
                RunStatus::Completed,
                "{:?}",
                result.error()
            );
            assert_eq!(result.step("read-token").output()["token"], "ghp_test");
        })
        .await
        .expect("test timed out");
    }

    #[cfg(feature = "secret-store")]
    #[tokio::test]
    async fn with_secret_last_value_wins() {
        timeout(Duration::from_secs(10), async {
            let result = TestEngine::new()
                .with_handler(UsesSecretOp)
                .with_secret("git_token", "first")
                .with_secret("git_token", "second")
                .run(json!({}))
                .await
                .unwrap();

            assert_eq!(result.step("read-token").output()["token"], "second");
        })
        .await
        .expect("test timed out");
    }

    #[cfg(feature = "secret-store")]
    #[tokio::test]
    async fn with_secret_is_readable_through_ctx_secrets() {
        timeout(Duration::from_secs(10), async {
            let mut harness = TestEngine::new()
                .with_handler(UsesSecretOp)
                .with_secret("k", "v");
            harness.run(json!({})).await.unwrap();

            let store = harness.store();
            let scope = |name: &str| {
                ScopedSecretStore::for_workflow(
                    Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()),
                    store.clone(),
                )
            };
            let own = scope("uses-secret-op").get("k").await.unwrap();
            assert_eq!(own.unwrap().value, "v");
            assert!(scope("other-workflow").get("k").await.unwrap().is_none());
        })
        .await
        .expect("test timed out");
    }

    #[cfg(feature = "secret-store")]
    #[tokio::test]
    #[should_panic(expected = "configure the TestEngine before its first run")]
    async fn with_secret_after_first_run_panics() {
        let mut harness = TestEngine::new().with_handler(UsesSecretOp);
        harness.run(json!({})).await.unwrap();
        let _ = harness.with_secret("git_token", "late");
    }
}
