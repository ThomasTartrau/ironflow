//! API server: owns persistence, serves the REST API and the dashboard,
//! and never executes a workflow itself. Workers do that.
//!
//! ```sh
//! cargo run -p server
//! ```
//!
//! Configuration comes from the environment (see `.env.example`).

use std::process;
use std::sync::Arc;

use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use ironflow_api::config::ServerConfig;
use ironflow_api::reaper::Reaper;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::sse::SseBroadcaster;
use ironflow_api::state::AppState;
use ironflow_artifacts::blob_store::BlobStore;
use ironflow_artifacts::local::LocalBlobStore;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::artifact::DirectArtifactSink;
use ironflow_engine::budget::BudgetConfig;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::{Event, WebhookSubscriber, WorkflowEventBus};
use ironflow_store::crypto::{KeyRing, SECRET_KEYS_ENV};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;

use workflows::handlers;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
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

    // ---- Store ------------------------------------------------------------
    // In-memory: runs are lost on restart. See the setup skill's
    // `references/options.md` to switch to Postgres.
    let mut store = InMemoryStore::new();

    let key_ring = KeyRing::from_env().unwrap_or_else(|e| {
        eprintln!("invalid secret key configuration: {e}");
        process::exit(1);
    });
    let has_key_ring = key_ring.is_some();
    match key_ring {
        Some(ring) => {
            info!(active_version = ring.active_version(), "secret store enabled");
            store.set_key_ring(ring);
        }
        None => info!("{SECRET_KEYS_ENV} not set, secret store disabled"),
    }

    let store: Arc<dyn Store> = Arc::new(store);

    // A secret encrypted with a key that is no longer configured is
    // unreadable. Fail here rather than at the first workflow that needs it.
    if has_key_ring {
        let status = store.secret_key_status().await.unwrap_or_else(|e| {
            eprintln!("cannot read secret key versions: {e}");
            process::exit(1);
        });
        if !status.is_consistent() {
            let missing: Vec<String> = status.missing.iter().map(|v| v.to_string()).collect();
            eprintln!(
                "secret key versions present in database but missing from configuration: {}",
                missing.join(", ")
            );
            process::exit(1);
        }
    }

    // ---- Engine -----------------------------------------------------------
    // The server never runs agent steps; the provider is only required by
    // the engine constructor. Workers choose the real provider.
    let provider = Arc::new(ClaudeCodeProvider::new());
    let budget = BudgetConfig::from_env();
    let mut engine = Engine::new(store.clone(), provider).with_budget_config(budget);
    for handler in handlers() {
        engine.register(handler).expect("handler names are unique");
    }

    // ---- Artifacts (optional) --------------------------------------------
    let blob_store: Option<Arc<dyn BlobStore>> = config.artifacts_dir.as_ref().map(|dir| {
        info!(dir = %dir.display(), "artifact storage enabled");
        Arc::new(LocalBlobStore::new(dir).max_bytes(config.artifact_max_bytes))
            as Arc<dyn BlobStore>
    });
    if let Some(ref blob) = blob_store {
        engine.set_artifact_sink(Arc::new(DirectArtifactSink::new(
            blob.clone(),
            store.clone(),
        )));
    }

    // ---- Notifications and live events -----------------------------------
    if let Some(ref webhook_url) = config.webhook_url {
        info!(url = %webhook_url, "registering webhook subscriber");
        engine.subscribe(
            WebhookSubscriber::new(webhook_url),
            &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
        );
    }
    let sse_broadcaster = SseBroadcaster::new();
    let event_sender = sse_broadcaster.sender();
    engine.subscribe(sse_broadcaster, Event::ALL);
    let event_bus = WorkflowEventBus::new();
    engine.set_event_bus(event_bus.clone());
    let engine = Arc::new(engine);

    // ---- HTTP -------------------------------------------------------------
    let jwt_config = Arc::new(JwtConfig {
        secret: config.jwt_secret.clone(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: config.is_production,
    });
    let mut state = AppState::new(
        store.clone(),
        engine.clone(),
        jwt_config,
        config.worker_token.clone(),
        event_sender,
    )
    .with_event_bus(event_bus);
    if let Some(blob) = blob_store {
        state = state.with_blob_store(blob);
    }

    // Without the reaper, a run whose worker dies stays Running forever.
    let shutdown = CancellationToken::new();
    spawn(Reaper::new(store, engine).run(shutdown.clone()));

    let router_config = RouterConfig {
        dashboard_dir: config.dashboard_dir.clone(),
        rate_limit_auth: config.rate_limit_auth,
        rate_limit_general: config.rate_limit_general,
    };
    let app = create_router(state, router_config)
        .layer(build_cors(&config))
        .into_make_service();

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr).await.expect("bind address");
    info!("ironflow server on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.expect("ctrl+c handler");
            info!("shutting down...");
            shutdown.cancel();
        })
        .await
        .expect("serve");
}

/// CORS: only the configured origins, or same-origin when none is set.
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
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(methods)
                .allow_headers(headers)
                .allow_credentials(true)
        }
        None => CorsLayer::new()
            .allow_methods(methods)
            .allow_headers(headers),
    }
}
