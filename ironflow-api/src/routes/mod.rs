//! Router assembly — one module per route.

pub mod api_keys;
pub mod approve_run;
pub mod audit_logs;
pub mod auth;
pub mod cancel_run;
pub mod create_run;
pub mod download_artifact;
pub mod events;
pub mod get_run;
pub mod get_run_logs;
pub mod get_stats;
pub mod get_workflow;
pub mod health_check;
mod internal;
pub mod list_runs;
pub mod list_workflows;
#[cfg(feature = "prometheus")]
pub mod metrics;
pub mod openapi_spec;
pub mod retry_run;
pub mod secrets;
#[cfg(test)]
mod test_helpers;
pub mod users;

use std::path::PathBuf;

use axum::Extension;
use axum::Router;
use axum::middleware as axum_mw;
use axum::routing::{delete, get, patch, post, put};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::middleware::{WorkerToken, security_headers, worker_token_auth};
use crate::rate_limit::{RateLimitContext, per_minute, rate_limit};
use crate::state::AppState;

/// Maximum request body size: 2 MiB.
const MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Maximum artifact upload size: 128 MiB.
///
/// A transport ceiling only, sitting slightly above the blob store's own limit
/// so an oversized payload is refused by the store with a precise error instead
/// of being cut off by the transport layer.
const MAX_ARTIFACT_BODY_SIZE: usize = 128 * 1024 * 1024;

/// Router-level configuration with sensible defaults.
///
/// Controls dashboard serving, rate limiting, and other router behaviors.
/// Use [`Default::default()`] for production-ready defaults, then override
/// individual fields as needed.
///
/// # Examples
///
/// ```
/// use ironflow_api::routes::RouterConfig;
///
/// // All defaults: rate limiting enabled, no custom dashboard dir
/// let config = RouterConfig::default();
/// assert_eq!(config.rate_limit_auth, Some(10));
///
/// // Disable auth rate limiting, custom dashboard
/// let config = RouterConfig {
///     rate_limit_auth: None,
///     ..RouterConfig::default()
/// };
/// ```
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Filesystem path to dashboard assets. When set, serves the SPA
    /// from this directory instead of the embedded build.
    pub dashboard_dir: Option<PathBuf>,
    /// Rate limit for auth credential routes (sign-in, sign-up) in
    /// requests per minute per IP. `None` disables the limiter.
    pub rate_limit_auth: Option<u32>,
    /// Rate limit for general public API routes in requests per minute
    /// per IP. `None` disables the limiter.
    pub rate_limit_general: Option<u32>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            dashboard_dir: None,
            rate_limit_auth: Some(10),
            rate_limit_general: Some(60),
        }
    }
}

/// Handler that returns a JSON 404 when the `sign-up` feature is disabled.
#[cfg(not(feature = "sign-up"))]
async fn sign_up_disabled() -> impl axum::response::IntoResponse {
    crate::error::ApiError::BadRequest("sign-up is disabled".to_string())
}

/// Create the main application router.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::routes::{RouterConfig, create_router};
/// use ironflow_api::state::AppState;
/// use ironflow_auth::jwt::JwtConfig;
/// use ironflow_store::prelude::*;
/// use ironflow_engine::engine::Engine;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use std::sync::Arc;
/// use tokio::sync::broadcast;
/// use ironflow_engine::notify::Event;
///
/// # async fn example() {
/// let store: Arc<dyn ironflow_store::store::Store> = Arc::new(InMemoryStore::new());
/// let provider = Arc::new(ClaudeCodeProvider::new());
/// let engine = Arc::new(Engine::new(store.clone(), provider));
/// let jwt_config = Arc::new(JwtConfig {
///     secret: "secret".to_string(),
///     access_token_ttl_secs: 900,
///     refresh_token_ttl_secs: 604800,
///     cookie_domain: None,
///     cookie_secure: false,
/// });
/// let (event_sender, _) = broadcast::channel::<Event>(1);
/// let state = AppState::new(store, engine, jwt_config, "token".to_string(), event_sender);
/// let router = create_router(state, RouterConfig::default());
/// # }
/// ```
pub fn create_router(state: AppState, config: RouterConfig) -> Router {
    // Internal routes (worker-to-API, protected by WORKER_TOKEN)
    let internal_routes = Router::new()
        .route("/runs", post(internal::create_run::create_run))
        .route("/runs/next", get(internal::pick_next_run::pick_next_run))
        .route(
            "/runs/{id}",
            get(internal::get_run::get_run).put(internal::update_run::update_run),
        )
        .route(
            "/runs/{id}/status",
            put(internal::update_run_status::update_run_status),
        )
        .route("/runs/{id}/logs", post(internal::push_logs::push_logs))
        .route("/runs/{id}/lease", post(internal::renew_lease::renew_lease))
        .route(
            "/runs/{id}/artifacts",
            get(internal::list_artifacts::list_artifacts),
        )
        .route("/steps", post(internal::create_step::create_step))
        .route("/steps/{id}", put(internal::update_step::update_step))
        .route(
            "/step-dependencies",
            post(internal::create_step_dependencies::create_step_dependencies),
        )
        .route("/secrets/{*key}", get(internal::get_secret::get_secret))
        .layer(axum_mw::from_fn(worker_token_auth))
        .layer(Extension(WorkerToken(state.worker_token.clone())))
        .with_state(state.clone());

    // Artifact uploads carry file payloads, so they sit outside the 2 MiB
    // limit that guards every JSON route and get their own, larger ceiling.
    let artifact_upload_routes = Router::new()
        .route(
            "/api/v1/internal/runs/{id}/steps/{step_id}/artifacts/{name}",
            post(internal::upload_artifact::upload_artifact),
        )
        .layer(axum_mw::from_fn(worker_token_auth))
        .layer(Extension(WorkerToken(state.worker_token.clone())))
        .layer(RequestBodyLimitLayer::new(MAX_ARTIFACT_BODY_SIZE))
        .with_state(state.clone());

    // Auth credential routes (rate-limited when configured)
    #[allow(unused_mut)]
    let mut auth_credential_routes = Router::new();

    #[cfg(feature = "sign-up")]
    {
        auth_credential_routes =
            auth_credential_routes.route("/sign-up", post(auth::sign_up::sign_up));
    }

    #[cfg(not(feature = "sign-up"))]
    {
        auth_credential_routes = auth_credential_routes.route("/sign-up", post(sign_up_disabled));
    }

    let mut auth_credential_routes =
        auth_credential_routes.route("/sign-in", post(auth::sign_in::sign_in));

    if let Some(rpm) = config.rate_limit_auth {
        let ctx = RateLimitContext {
            store: state.store.clone(),
            jwt_config: state.jwt_config.clone(),
            limiter: per_minute(rpm),
        };
        auth_credential_routes = auth_credential_routes
            .layer(axum_mw::from_fn(rate_limit))
            .layer(Extension(ctx));
    }

    // Auth session routes (no strict rate limiting, covered by general limiter)
    let auth_session_routes = Router::new()
        .route("/refresh", post(auth::refresh::refresh))
        .route("/sign-out", post(auth::sign_out::sign_out))
        .route("/me", get(auth::me::me));

    // Public + user-authenticated routes (rate-limited when configured)
    #[allow(unused_mut)]
    let mut api_v1 = Router::new()
        .route("/health-check", get(health_check::health_check))
        .route("/openapi.json", get(openapi_spec::openapi_spec))
        .route(
            "/runs",
            get(list_runs::list_runs).post(create_run::create_run),
        )
        .route("/runs/{id}", get(get_run::get_run))
        .route("/runs/{id}/logs", get(get_run_logs::get_run_logs))
        .route("/runs/{id}/cancel", post(cancel_run::cancel_run))
        .route("/runs/{id}/approve", post(approve_run::approve_run))
        .route("/runs/{id}/reject", post(approve_run::reject_run))
        .route("/runs/{id}/retry", post(retry_run::retry_run))
        .route(
            "/runs/{id}/steps/{step_id}/artifacts/{name}",
            get(download_artifact::download_artifact),
        )
        .route("/workflows", get(list_workflows::list_workflows))
        .route("/workflows/{name}", get(get_workflow::get_workflow))
        .route("/stats", get(get_stats::get_stats))
        .route("/audit-logs", get(audit_logs::list_audit_logs))
        .route("/events", get(events::events))
        .route(
            "/api-keys",
            get(api_keys::list::list_api_keys).post(api_keys::create::create_api_key),
        )
        .route(
            "/api-keys/scopes",
            get(api_keys::available_scopes::available_scopes),
        )
        .route("/api-keys/{id}", delete(api_keys::delete::delete_api_key))
        .route(
            "/users",
            get(users::list::list_users).post(users::create::create_user),
        )
        .route("/users/{id}", delete(users::delete::delete_user))
        .route("/users/{id}/role", patch(users::update_role::update_role))
        .route(
            "/secrets",
            get(secrets::list::list_secrets).post(secrets::create::create_secret),
        )
        // Registered before the catch-all below: a static segment wins over
        // the wildcard, so `rotate` is never read as a secret key.
        .route("/secrets/rotate", post(secrets::rotate::rotate_secrets))
        .route(
            "/secrets/key-versions",
            get(secrets::key_versions::secret_key_versions),
        )
        .route(
            "/secrets/{*key}",
            put(secrets::update::update_secret).delete(secrets::delete::delete_secret),
        );

    #[cfg(feature = "prometheus")]
    {
        api_v1 = api_v1.route("/metrics", get(metrics::metrics));
    }

    let mut api_v1 = api_v1
        .nest("/auth", auth_credential_routes)
        .nest("/auth", auth_session_routes);

    if let Some(rpm) = config.rate_limit_general {
        let ctx = RateLimitContext {
            store: state.store.clone(),
            jwt_config: state.jwt_config.clone(),
            limiter: per_minute(rpm),
        };
        api_v1 = api_v1
            .layer(axum_mw::from_fn(rate_limit))
            .layer(Extension(ctx));
    }

    let api_v1 = api_v1.with_state(state.clone());

    #[allow(unused_mut)]
    let mut app = Router::new()
        .nest("/api/v1/internal", internal_routes)
        .nest("/api/v1", api_v1)
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .merge(artifact_upload_routes)
        .layer(axum_mw::from_fn(security_headers));

    #[cfg(feature = "prometheus")]
    {
        app = app.layer(axum_mw::from_fn(crate::middleware::request_metrics));
    }

    match config.dashboard_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            let serve = ServeDir::new(dir).fallback(ServeFile::new(index));
            app.fallback_service(serve)
        }
        #[cfg(feature = "dashboard")]
        None => app.fallback_service(crate::dashboard::EmbeddedDashboard),
        #[cfg(not(feature = "dashboard"))]
        None => app,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
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

    #[tokio::test]
    async fn health_check_route() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/health-check")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"OK");
    }

    fn make_auth_header(state: &AppState) -> String {
        use ironflow_auth::jwt::AccessToken;
        use uuid::Uuid;

        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn runs_route_exists() {
        let state = test_state();
        let app = create_router(state.clone(), RouterConfig::default());
        let auth_header = make_auth_header(&state);

        let req = Request::builder()
            .uri("/api/v1/runs?page=1&per_page=20")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn stats_route_exists() {
        let state = test_state();
        let app = create_router(state.clone(), RouterConfig::default());
        let auth_header = make_auth_header(&state);

        let req = Request::builder()
            .uri("/api/v1/stats")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn responses_include_security_headers() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/health-check")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
        assert_eq!(
            resp.headers().get("x-xss-protection").unwrap(),
            "1; mode=block"
        );
        assert_eq!(
            resp.headers().get("strict-transport-security").unwrap(),
            "max-age=63072000; includeSubDomains"
        );
        assert!(
            resp.headers()
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("default-src 'self'")
        );
    }

    #[tokio::test]
    async fn body_size_limit_rejects_oversized_payload() {
        let state = test_state();
        let app = create_router(state.clone(), RouterConfig::default());
        let auth_header = make_auth_header(&state);

        // 3 MiB payload — exceeds the 2 MiB limit
        let oversized = vec![0u8; 3 * 1024 * 1024];

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/runs")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(oversized))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
