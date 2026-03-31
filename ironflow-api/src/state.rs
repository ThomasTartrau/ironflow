//! Application state and dependency injection.
//!
//! [`AppState`] holds the shared [`RunStore`] and [`Engine`] used by all handlers.

use axum::extract::FromRef;
use ironflow_auth::jwt::JwtConfig;
use ironflow_engine::engine::Engine;
use ironflow_store::entities::Run;
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ApiError;

/// Global application state.
///
/// Holds the shared run store and engine, extracted by handlers using Axum's
/// state extraction mechanism.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::state::AppState;
/// use ironflow_auth::jwt::JwtConfig;
/// use ironflow_store::prelude::*;
/// use ironflow_engine::engine::Engine;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use std::sync::Arc;
///
/// # async fn example() {
/// let store = Arc::new(InMemoryStore::new());
/// let user_store: Arc<dyn UserStore> = Arc::new(InMemoryStore::new());
/// let provider = Arc::new(ClaudeCodeProvider::new());
/// let engine = Arc::new(Engine::new(store.clone(), provider));
/// let jwt_config = Arc::new(JwtConfig {
///     secret: "secret".to_string(),
///     access_token_ttl_secs: 900,
///     refresh_token_ttl_secs: 604800,
///     cookie_domain: None,
///     cookie_secure: false,
/// });
/// let state = AppState { store, user_store, engine, jwt_config, worker_token: "token".to_string() };
/// # }
/// ```
#[derive(Clone)]
pub struct AppState {
    /// The backing store for runs and steps.
    pub store: Arc<dyn RunStore>,
    /// The backing store for users.
    pub user_store: Arc<dyn UserStore>,
    /// The workflow orchestration engine.
    pub engine: Arc<Engine>,
    /// JWT configuration for auth tokens.
    pub jwt_config: Arc<JwtConfig>,
    /// Static token for worker-to-API authentication.
    pub worker_token: String,
}

impl FromRef<AppState> for Arc<dyn RunStore> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.store)
    }
}

impl FromRef<AppState> for Arc<dyn UserStore> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.user_store)
    }
}

impl FromRef<AppState> for Arc<JwtConfig> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.jwt_config)
    }
}

impl AppState {
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

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        AppState {
            store,
            user_store,
            engine,
            jwt_config,
            worker_token: "test-worker-token".to_string(),
        }
    }

    #[test]
    fn app_state_cloneable() {
        let state = test_state();
        let _cloned = state.clone();
    }

    #[test]
    fn app_state_from_ref() {
        let state = test_state();
        let extracted: Arc<dyn RunStore> = Arc::from_ref(&state);
        assert!(Arc::ptr_eq(&extracted, &state.store));
    }
}
