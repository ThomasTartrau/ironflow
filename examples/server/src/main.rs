//! ironflow API server example.
//!
//! ```sh
//! cargo run -p ironflow-example-server
//! ```
//!
//! The dashboard is served automatically via the `dashboard` feature in `ironflow-api`.
//!
//! Environment:
//! - `JWT_SECRET` (default: dev secret)
//! - `WORKER_TOKEN` (default: dev token)
//! - `PORT` (default: 3000)
//! - `DASHBOARD_DIR` (optional: overrides the embedded dashboard with a filesystem path)
//! - `ALLOWED_ORIGINS` (comma-separated list; omit to allow same-origin only)

use std::sync::Arc;

use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use ironflow_api::routes::create_router;
use ironflow_api::state::AppState;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ironflow=debug".parse().expect("valid filter")),
        )
        .init();

    let store: Arc<dyn RunStore> = Arc::new(InMemoryStore::new());
    let user_store: Arc<dyn UserStore> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());

    let jwt_config = Arc::new(JwtConfig {
        secret: std::env::var("JWT_SECRET").unwrap_or_else(|_| {
            warn!("JWT_SECRET not set, using insecure dev default — do NOT use in production");
            "ironflow-dev-secret".to_string()
        }),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let worker_token = std::env::var("WORKER_TOKEN").unwrap_or_else(|_| {
        warn!("WORKER_TOKEN not set, using insecure dev default — do NOT use in production");
        "ironflow-dev-worker-token".to_string()
    });
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let mut engine = Engine::new(store.clone(), provider);
    ironflow_workflows::register_all(&mut engine).expect("failed to register workflows");
    let engine = Arc::new(engine);

    let cors = build_cors();

    let dashboard_dir = std::env::var("DASHBOARD_DIR")
        .ok()
        .map(std::path::PathBuf::from);

    let state = AppState {
        store,
        user_store,
        engine,
        jwt_config,
        worker_token,
    };
    let app = create_router(state, dashboard_dir)
        .layer(cors)
        .into_make_service();

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind address");

    info!("==============================================");
    info!("  ironflow server on http://{addr}");
    info!("==============================================");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.expect("ctrl+c handler");
            info!("shutting down...");
        })
        .await
        .expect("serve");
}

/// Build CORS layer from `ALLOWED_ORIGINS` env var.
///
/// - If `ALLOWED_ORIGINS` is set: only those origins are permitted (comma-separated).
/// - If unset: no extra origins are allowed (same-origin only).
///
/// Credentials (cookies) are always allowed so JWT cookies work cross-origin.
fn build_cors() -> CorsLayer {
    let methods = vec![Method::GET, Method::POST, Method::PUT, Method::DELETE];
    let headers = vec![AUTHORIZATION, CONTENT_TYPE];

    match std::env::var("ALLOWED_ORIGINS") {
        Ok(raw) => {
            let origins: Vec<HeaderValue> = raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .filter_map(|s| match s.parse::<HeaderValue>() {
                    Ok(v) => Some(v),
                    Err(err) => {
                        warn!(origin = s, %err, "ignoring invalid CORS origin");
                        None
                    }
                })
                .collect();

            info!(?origins, "CORS: allowing configured origins");

            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(methods)
                .allow_headers(headers)
                .allow_credentials(true)
        }
        Err(_) => {
            info!("CORS: no ALLOWED_ORIGINS set, same-origin only");

            CorsLayer::new()
                .allow_methods(methods)
                .allow_headers(headers)
        }
    }
}
