//! ironflow API server example.
//!
//! ```sh
//! cargo run -p ironflow-example-server
//! ```
//!
//! The dashboard is served automatically via the `dashboard` feature in `ironflow-api`.
//!
//! Environment:
//! - `IRONFLOW_ENV` (`production` or `development`, default: development)
//! - `DATABASE_URL` (required in production)
//! - `JWT_SECRET` (required in production, default: dev secret)
//! - `WORKER_TOKEN` (required in production, default: dev token)
//! - `PORT` (default: 3000)
//! - `DASHBOARD_DIR` (optional: overrides the embedded dashboard with a filesystem path)
//! - `ALLOWED_ORIGINS` (comma-separated list; omit to allow same-origin only)
//! - `WEBHOOK_URL` (optional: outbound webhook for run events)

use std::process;
use std::sync::Arc;

use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use ironflow_api::config::ServerConfig;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::{Event, WebhookSubscriber};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ironflow=debug".parse().expect("valid filter")),
        )
        .init();

    let config = ServerConfig::from_env().unwrap_or_else(|e| {
        eprintln!("{e}");
        process::exit(1);
    });

    let store: Arc<dyn RunStore> = Arc::new(InMemoryStore::new());
    let user_store: Arc<dyn UserStore> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());

    let jwt_config = Arc::new(JwtConfig {
        secret: config.jwt_secret.clone(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: config.is_production,
    });

    let mut engine = Engine::new(store.clone(), provider);
    ironflow_workflows::register_all(&mut engine).expect("failed to register workflows");

    if let Some(ref webhook_url) = config.webhook_url {
        info!(url = %webhook_url, "registering webhook subscriber");
        engine.subscribe(
            WebhookSubscriber::new(webhook_url),
            &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
        );
    }

    let engine = Arc::new(engine);

    let cors = build_cors(&config);

    let state = AppState::new(
        store,
        user_store,
        engine,
        jwt_config,
        config.worker_token.clone(),
    );
    let router_config = RouterConfig {
        dashboard_dir: config.dashboard_dir.clone(),
        rate_limit_auth: config.rate_limit_auth,
        rate_limit_general: config.rate_limit_general,
    };
    let app = create_router(state, router_config)
        .layer(cors)
        .into_make_service();

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr).await.expect("bind address");

    info!("==============================================");
    info!("  ironflow server on http://{addr}");
    info!(
        "  environment: {}",
        if config.is_production {
            "production"
        } else {
            "development"
        }
    );
    info!("==============================================");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.expect("ctrl+c handler");
            info!("shutting down...");
        })
        .await
        .expect("serve");
}

/// Build CORS layer from config.
///
/// - If `allowed_origins` is set: only those origins are permitted (comma-separated).
/// - If unset: no extra origins are allowed (same-origin only).
///
/// Credentials (cookies) are always allowed so JWT cookies work cross-origin.
fn build_cors(config: &ServerConfig) -> CorsLayer {
    let methods = vec![Method::GET, Method::POST, Method::PUT, Method::DELETE];
    let headers = vec![AUTHORIZATION, CONTENT_TYPE];

    match config.allowed_origins {
        Some(ref raw) => {
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
        None => {
            info!("CORS: no ALLOWED_ORIGINS set, same-origin only");

            CorsLayer::new()
                .allow_methods(methods)
                .allow_headers(headers)
        }
    }
}
