//! Application state and dependency injection.
//!
//! [`AppState`] holds the shared [`Store`] and [`Engine`] used by all handlers.

use std::sync::Arc;
#[cfg(feature = "prometheus")]
use std::sync::OnceLock;

use axum::extract::FromRef;
#[cfg(feature = "prometheus")]
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tokio::sync::broadcast;
use uuid::Uuid;

use ironflow_auth::jwt::JwtConfig;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::Run;
use ironflow_store::store::Store;

use crate::error::ApiError;

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
    /// Prometheus metrics handle (only when `prometheus` feature is enabled).
    #[cfg(feature = "prometheus")]
    pub prometheus_handle: PrometheusHandle,
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
            #[cfg(feature = "prometheus")]
            prometheus_handle: Self::global_prometheus_handle(),
        }
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
