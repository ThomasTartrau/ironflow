//! Router assembly — one module per route.

mod auth;
mod cancel_run;
mod create_run;
mod get_run;
mod get_stats;
mod get_workflow;
mod health_check;
mod internal;
mod list_runs;
mod list_workflows;
mod retry_run;

use std::path::PathBuf;

use axum::Extension;
use axum::Router;
use axum::middleware as axum_mw;
use axum::routing::{get, post, put};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::middleware::{WorkerToken, security_headers, worker_token_auth};
use crate::state::AppState;

/// Maximum request body size: 2 MiB.
const MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Create the main application router.
///
/// If `dashboard_dir` is provided, the router serves the SPA dashboard from
/// that directory. Any request that doesn't match an API route is served from
/// the directory, with a fallback to `index.html` for client-side routing.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::routes::create_router;
/// use ironflow_api::state::AppState;
/// use ironflow_auth::jwt::JwtConfig;
/// use ironflow_store::prelude::*;
/// use ironflow_engine::engine::Engine;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use std::sync::Arc;
/// use std::path::PathBuf;
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
/// let router = create_router(state, None);
/// # }
/// ```
pub fn create_router(state: AppState, dashboard_dir: Option<PathBuf>) -> Router {
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
        .route("/steps", post(internal::create_step::create_step))
        .route("/steps/{id}", put(internal::update_step::update_step))
        .route(
            "/step-dependencies",
            post(internal::create_step_dependencies::create_step_dependencies),
        )
        .layer(axum_mw::from_fn(worker_token_auth))
        .layer(Extension(WorkerToken(state.worker_token.clone())))
        .with_state(state.clone());

    // Public + user-authenticated routes
    let api_v1 = Router::new()
        .route("/health-check", get(health_check::health_check))
        .route(
            "/runs",
            get(list_runs::list_runs).post(create_run::create_run),
        )
        .route("/runs/{id}", get(get_run::get_run))
        .route("/runs/{id}/cancel", post(cancel_run::cancel_run))
        .route("/runs/{id}/retry", post(retry_run::retry_run))
        .route("/workflows", get(list_workflows::list_workflows))
        .route("/workflows/{name}", get(get_workflow::get_workflow))
        .route("/stats", get(get_stats::get_stats));

    #[cfg(feature = "sign-up")]
    let api_v1 = api_v1.route("/auth/sign-up", post(auth::sign_up::sign_up));

    let api_v1 = api_v1
        .route("/auth/sign-in", post(auth::sign_in::sign_in))
        .route("/auth/refresh", post(auth::refresh::refresh))
        .route("/auth/sign-out", post(auth::sign_out::sign_out))
        .route("/auth/me", get(auth::me::me))
        .with_state(state.clone());

    let app = Router::new()
        .nest("/api/v1/internal", internal_routes)
        .nest("/api/v1", api_v1)
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .layer(axum_mw::from_fn(security_headers));

    match dashboard_dir {
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
    use ironflow_store::memory::InMemoryStore;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
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

    #[tokio::test]
    async fn health_check_route() {
        let state = test_state();
        let app = create_router(state, None);

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
        let app = create_router(state.clone(), None);
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
        let app = create_router(state.clone(), None);
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
        let app = create_router(state, None);

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
        let app = create_router(state.clone(), None);
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
