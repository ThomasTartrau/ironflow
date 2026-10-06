//! Application state and dependency injection.
//!
//! [`AppState`] holds the shared [`Store`] and [`Engine`] used by all handlers.

use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "prometheus")]
use std::sync::OnceLock;

use axum::extract::FromRef;
#[cfg(feature = "prometheus")]
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tokio::sync::broadcast;
use uuid::Uuid;

use tokio_util::sync::CancellationToken;
use tracing::warn;

use ironflow_artifacts::blob_store::BlobStore;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::account::{AccountKind, ClaudeSubscriptionKind};
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::{Event, WorkflowEventBus};
use ironflow_store::entities::Run;
use ironflow_store::store::Store;

use crate::error::ApiError;
use crate::escalator::Escalator;
use crate::reaper::Reaper;
use crate::schedule_sync::{repair_unscheduled_schedules, sync_handler_schedules};
use crate::schedule_ticker::ScheduleTicker;
use crate::waker::Waker;

/// Global application state.
///
/// Holds the shared store (runs, users, API keys, secrets) and engine,
/// extracted by handlers using Axum's state extraction mechanism.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::state::AppState;
/// use ironflow_auth::jwt::JwtConfig;
/// use ironflow_store::prelude::*;
/// use ironflow_store::store::Store;
/// use ironflow_engine::engine::Engine;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use std::sync::Arc;
///
/// # async fn example() {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let provider = Arc::new(ClaudeCodeProvider::new());
/// let engine = Arc::new(Engine::new(store.clone(), provider));
/// let jwt_config = Arc::new(JwtConfig {
///     secret: "secret".to_string(),
///     access_token_ttl_secs: 900,
///     refresh_token_ttl_secs: 604800,
///     cookie_domain: None,
///     cookie_secure: false,
/// });
/// let broadcaster = ironflow_api::sse::SseBroadcaster::new();
/// let state = AppState::new(store, engine, jwt_config, "token".to_string(), broadcaster.sender());
/// # }
/// ```
#[derive(Clone)]
pub struct AppState {
    /// The unified backing store for runs, steps, users, API keys, and secrets.
    pub store: Arc<dyn Store>,
    /// The workflow orchestration engine.
    pub engine: Arc<Engine>,
    /// JWT configuration for auth tokens.
    pub jwt_config: Arc<JwtConfig>,
    /// Static token for worker-to-API authentication.
    pub worker_token: String,
    /// Broadcast sender for SSE event streaming.
    pub event_sender: broadcast::Sender<Event>,
    /// Per-run event bus for real-time workflow monitoring.
    ///
    /// When set, the `GET /api/v1/runs/{id}/events` route subscribes to this
    /// bus and streams [`WorkflowEvent`](ironflow_engine::notify::WorkflowEvent)s
    /// via SSE. `None` when the engine was not configured with a bus.
    pub event_bus: Option<WorkflowEventBus>,
    /// Where artifact bytes live, when artifacts are enabled.
    ///
    /// `None` on a deployment that has not configured artifact storage: the
    /// artifact routes answer `501` and every other endpoint is unaffected.
    pub blob_store: Option<Arc<dyn BlobStore>>,
    /// Registered Provider Account kinds, by id.
    pub account_kinds: Arc<HashMap<&'static str, Arc<dyn AccountKind>>>,
    /// Prometheus metrics handle (only when `prometheus` feature is enabled).
    #[cfg(feature = "prometheus")]
    pub prometheus_handle: PrometheusHandle,
}

/// The kinds every server supports.
fn default_account_kinds() -> HashMap<&'static str, Arc<dyn AccountKind>> {
    let claude: Arc<dyn AccountKind> = Arc::new(ClaudeSubscriptionKind::new());
    HashMap::from([(claude.id(), claude)])
}

impl FromRef<AppState> for Arc<dyn Store> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.store)
    }
}

impl FromRef<AppState> for Arc<JwtConfig> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.jwt_config)
    }
}

#[cfg(feature = "prometheus")]
impl FromRef<AppState> for PrometheusHandle {
    fn from_ref(state: &AppState) -> Self {
        state.prometheus_handle.clone()
    }
}

impl AppState {
    /// Create a new `AppState`.
    ///
    /// When the `prometheus` feature is enabled, a global Prometheus recorder
    /// is installed (once) and its handle is stored in the state.
    ///
    /// # Panics
    ///
    /// Panics if a Prometheus recorder cannot be installed (should only
    /// happen if another incompatible recorder was set elsewhere).
    pub fn new(
        store: Arc<dyn Store>,
        engine: Arc<Engine>,
        jwt_config: Arc<JwtConfig>,
        worker_token: String,
        event_sender: broadcast::Sender<Event>,
    ) -> Self {
        Self {
            store,
            engine,
            jwt_config,
            worker_token,
            event_sender,
            event_bus: None,
            blob_store: None,
            account_kinds: Arc::new(default_account_kinds()),
            #[cfg(feature = "prometheus")]
            prometheus_handle: Self::global_prometheus_handle(),
        }
    }

    /// Enable artifacts by attaching the backend that holds their bytes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use ironflow_api::state::AppState;
    /// use ironflow_artifacts::blob_store::BlobStore;
    /// use ironflow_artifacts::local::LocalBlobStore;
    ///
    /// # fn example(state: AppState) -> AppState {
    /// let blob: Arc<dyn BlobStore> = Arc::new(LocalBlobStore::new("/var/lib/ironflow/artifacts"));
    /// state.with_blob_store(blob)
    /// # }
    /// ```
    pub fn with_blob_store(mut self, blob_store: Arc<dyn BlobStore>) -> Self {
        self.blob_store = Some(blob_store);
        self
    }

    /// Register a Provider Account kind, replacing one with the same id.
    ///
    /// Tests use it to point the Claude kind at a local stub server.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use ironflow_api::state::AppState;
    /// use ironflow_core::account::ClaudeSubscriptionKind;
    ///
    /// # fn example(state: AppState) -> AppState {
    /// state.with_account_kind(Arc::new(ClaudeSubscriptionKind::with_api_base("http://127.0.0.1:9999")))
    /// # }
    /// ```
    pub fn with_account_kind(mut self, kind: Arc<dyn AccountKind>) -> Self {
        let mut kinds = (*self.account_kinds).clone();
        kinds.insert(kind.id(), kind);
        self.account_kinds = Arc::new(kinds);
        self
    }

    /// Attach a [`WorkflowEventBus`] for per-run SSE streaming.
    ///
    /// When set, `GET /api/v1/runs/{id}/events` streams step-level events
    /// for a specific workflow run. When absent the route returns an empty
    /// SSE stream (with keep-alive).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_api::state::AppState;
    /// use ironflow_engine::notify::WorkflowEventBus;
    ///
    /// # fn example(state: AppState) -> AppState {
    /// state.with_event_bus(WorkflowEventBus::new())
    /// # }
    /// ```
    pub fn with_event_bus(mut self, bus: WorkflowEventBus) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// The artifact backend, or a `501` error when artifacts are disabled.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::ArtifactStorageUnavailable`] when no backend is attached.
    pub fn blob_store_or_501(&self) -> Result<&Arc<dyn BlobStore>, ApiError> {
        self.blob_store
            .as_ref()
            .ok_or(ApiError::ArtifactStorageUnavailable)
    }

    /// Install (or reuse) a global Prometheus recorder and return its handle.
    #[cfg(feature = "prometheus")]
    fn global_prometheus_handle() -> PrometheusHandle {
        static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
        HANDLE
            .get_or_init(|| {
                PrometheusBuilder::new()
                    .install_recorder()
                    .expect("failed to install Prometheus recorder")
            })
            .clone()
    }

    /// Fetch a run by ID or return 404.
    ///
    /// # Errors
    ///
    /// Returns `ApiError::RunNotFound` if the run does not exist.
    /// Returns `ApiError::Store` if there is a store error.
    pub async fn get_run_or_404(&self, id: Uuid) -> Result<Run, ApiError> {
        self.store
            .get_run(id)
            .await
            .map_err(ApiError::from)?
            .ok_or(ApiError::RunNotFound(id))
    }

    /// Spawn the built-in background tasks and return their shared shutdown token.
    ///
    /// This starts:
    /// - **Schedule sync**: seeds DB rows for handler-declared schedules.
    /// - **Schedule ticker**: polls due schedules and creates runs.
    /// - **Reaper**: recovers runs abandoned by dead workers.
    /// - **Escalator**: resolves approval gates that missed their SLA deadline.
    /// - **Waker**: wakes `Sleeping` runs whose delay elapsed or whose signal
    ///   wait timed out.
    ///
    /// Call this once after building the `AppState`, before serving requests.
    /// Drop the returned [`CancellationToken`] (or call `.cancel()`) to stop
    /// all tasks gracefully.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_api::state::AppState;
    ///
    /// # async fn example(state: AppState) {
    /// let shutdown = state.spawn_background_tasks().await;
    /// // ... serve requests ...
    /// shutdown.cancel();
    /// # }
    /// ```
    pub async fn spawn_background_tasks(&self) -> CancellationToken {
        if let Err(err) = sync_handler_schedules(&self.engine, self.store.as_ref()).await {
            warn!(error = %err, "failed to sync handler-declared schedules");
        }
        if let Err(err) = repair_unscheduled_schedules(self.store.as_ref()).await {
            warn!(error = %err, "failed to repair schedules without next trigger");
        }

        let shutdown = CancellationToken::new();
        tokio::spawn(ScheduleTicker::new(self.store.clone()).run(shutdown.clone()));
        tokio::spawn(Reaper::new(self.store.clone(), self.engine.clone()).run(shutdown.clone()));
        tokio::spawn(Escalator::new(self.engine.clone()).run(shutdown.clone()));
        tokio::spawn(Waker::new(self.engine.clone()).run(shutdown.clone()));
        shutdown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;

    fn test_state() -> AppState {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    #[test]
    fn app_state_cloneable() {
        let state = test_state();
        let _cloned = state.clone();
    }

    #[test]
    fn app_state_from_ref() {
        let state = test_state();
        let extracted: Arc<dyn Store> = Arc::from_ref(&state);
        assert!(Arc::ptr_eq(&extracted, &state.store));
    }
}
