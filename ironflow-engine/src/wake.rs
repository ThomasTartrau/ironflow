//! [`RunWaker`] -- wakes `Sleeping` runs whose `scheduled_at` has passed.
//!
//! A run goes `Sleeping` when a [`delay`](crate::context::WorkflowContext::delay)
//! step pauses it, or when
//! [`wait_for_signal`](crate::context::WorkflowContext::wait_for_signal) suspends
//! it until a signal or its deadline. Either way the wake-up time lives on the
//! run itself (`scheduled_at`), so the timer survives an API or worker restart:
//! nothing is held in memory, and a brand-new process picks the run up on its
//! next tick.
//!
//! [`RunWaker::tick`] claims every due run and moves it back to `Pending` in
//! the same transaction that clears `scheduled_at`, so a run wakes **exactly
//! once** even with several API instances running a waker. Under
//! [`ExecutionMode::Workers`] a worker then picks the run up; under
//! [`ExecutionMode::Local`] the waker resumes it in-process.

use std::sync::Arc;

use tracing::info;

use ironflow_store::models::Run;

use crate::engine::{Engine, ExecutionMode};
use crate::error::EngineError;

/// How many runs a single [`RunWaker::tick`] wakes.
pub const DEFAULT_WAKE_BATCH_SIZE: u32 = 50;

/// Wakes `Sleeping` runs whose wake-up time has passed.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use ironflow_engine::engine::Engine;
/// use ironflow_engine::wake::RunWaker;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
///
/// # async fn example() -> Result<(), ironflow_engine::error::EngineError> {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
///
/// let waker = RunWaker::new(engine).batch_size(10);
/// let woken = waker.tick().await?;
/// println!("{} runs woken", woken.len());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RunWaker {
    engine: Arc<Engine>,
    batch_size: u32,
}

impl RunWaker {
    /// Create a waker with the default batch size.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::wake::RunWaker;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::store::Store;
    ///
    /// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    /// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
    /// let waker = RunWaker::new(engine);
    /// ```
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            engine,
            batch_size: DEFAULT_WAKE_BATCH_SIZE,
        }
    }

    /// Set how many runs a single [`tick`](Self::tick) wakes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::wake::RunWaker;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::store::Store;
    ///
    /// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    /// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
    /// let waker = RunWaker::new(engine).batch_size(10);
    /// ```
    pub fn batch_size(mut self, batch_size: u32) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Claim and wake one batch of due `Sleeping` runs.
    ///
    /// Every claimed run is `Pending` when this returns. Under
    /// [`ExecutionMode::Local`] each one is then resumed in a background task,
    /// so a long workflow never holds the waker loop; a failed resume is
    /// logged, not rolled back.
    ///
    /// Returns the woken runs as they were right after the transition.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] if the batch cannot be claimed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::wake::RunWaker;
    ///
    /// # use ironflow_engine::error::EngineError;
    /// # async fn example(waker: &RunWaker) -> Result<(), EngineError> {
    /// for run in waker.tick().await? {
    ///     println!("woke {}", run.id);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn tick(&self) -> Result<Vec<Run>, EngineError> {
        let runs = self
            .engine
            .store()
            .claim_due_sleeping_runs(self.batch_size)
            .await?;

        for run in &runs {
            info!(
                run_id = %run.id,
                workflow = %run.workflow_name,
                "sleeping run woken"
            );
            if self.engine.execution_mode() == ExecutionMode::Local {
                self.engine.spawn_local_resume(run.id);
            }
        }

        Ok(runs)
    }
}
