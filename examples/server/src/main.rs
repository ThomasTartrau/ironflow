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
//! - `ARTIFACTS_DIR` (optional: filesystem root for step artifacts; unset
//!   leaves artifacts disabled and the artifact routes answer `501`)
//! - `ARTIFACT_MAX_BYTES` (optional: per-artifact size limit, default 100 MiB)
//! - `IRONFLOW_DEFAULT_RUN_MAX_COST_USD` (optional: default per-run cost cap in
//!   USD, applied when neither the run creation request nor the workflow
//!   handler declares one; unset means no cap)
//! - `IRONFLOW_MONTHLY_COST_LIMIT_USD` (optional: global cost quota for the
//!   current calendar month in UTC; beyond it, creating a run returns
//!   `429 MONTHLY_BUDGET_EXCEEDED` while in-flight runs continue)

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
use ironflow_engine::notify::{Event, WebhookSubscriber};
use ironflow_store::crypto::{KeyRing, SECRET_KEYS_ENV};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;

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

    let mut store = InMemoryStore::new();

    let key_ring = KeyRing::from_env().unwrap_or_else(|e| {
        eprintln!("invalid secret key configuration: {e}");
        process::exit(1);
    });

    let has_key_ring = key_ring.is_some();
    match key_ring {
        Some(ring) => {
            info!(
                active_version = ring.active_version(),
                configured_versions = ?ring.versions(),
                "secret store enabled"
            );
            store.set_key_ring(ring);
        }
        None => {
            info!("{SECRET_KEYS_ENV} not set, secret store disabled");
        }
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
                "secret key versions present in database but missing from configuration: {}\n\
                 set {SECRET_KEYS_ENV} to include them, or rotate before removing a key",
                missing.join(", ")
            );
            process::exit(1);
        }
    }
    let provider = Arc::new(ClaudeCodeProvider::new());

    let jwt_config = Arc::new(JwtConfig {
        secret: config.jwt_secret.clone(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: config.is_production,
    });

    let budget = BudgetConfig::from_env();
    info!(
        default_run_max_cost_usd = ?budget.default_run_max_cost_usd,
        monthly_cost_limit_usd = ?budget.monthly_cost_limit_usd,
        "cost guardrails loaded"
    );

    let mut engine = Engine::new(store.clone(), provider).with_budget_config(budget);
    ironflow_workflows::register_all(&mut engine).expect("failed to register workflows");

    // Artifacts stay off until a storage root is configured. The API and any
    // in-process run then share the same backend, so a file a step produces is
    // downloadable from the same server that stored it.
    let blob_store: Option<Arc<dyn BlobStore>> = config.artifacts_dir.as_ref().map(|dir| {
        info!(
            dir = %dir.display(),
            max_bytes = config.artifact_max_bytes,
            "artifact storage enabled"
        );
        Arc::new(LocalBlobStore::new(dir).max_bytes(config.artifact_max_bytes))
            as Arc<dyn BlobStore>
    });

    if let Some(ref blob) = blob_store {
        engine.set_artifact_sink(Arc::new(DirectArtifactSink::new(
            blob.clone(),
            store.clone(),
        )));
    }

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

    let engine = Arc::new(engine);

    let cors = build_cors(&config);

    let mut state = AppState::new(
        store.clone(),
        engine.clone(),
        jwt_config,
        config.worker_token.clone(),
        event_sender,
    );
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
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.expect("ctrl+c handler");
            info!("shutting down...");
            shutdown.cancel();
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
