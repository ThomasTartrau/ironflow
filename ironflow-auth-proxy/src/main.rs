//! `ironflow-auth-proxy` binary.
//!
//! Environment:
//!
//! * `IRONFLOW_AUTH_PROXY_ADMIN_KEY` (required, at least 32 characters) - key
//!   of the admin API, shared with the worker.
//! * `IRONFLOW_AUTH_PROXY_LISTEN` (default `0.0.0.0:8080`) - listen address.
//! * `IRONFLOW_AUTH_PROXY_DATABASE_URL` (optional) - PostgreSQL URL of a
//!   token registry shared by several replicas and surviving restarts. Unset,
//!   tokens live in memory and a single replica must run.
//! * `IRONFLOW_SECRET_KEYS` / `IRONFLOW_SECRET_ACTIVE_KEY_VERSION` (or the
//!   legacy `IRONFLOW_SECRET_KEY`) - key ring encrypting the credentials at
//!   rest. Required with `IRONFLOW_AUTH_PROXY_DATABASE_URL`.
//! * `RUST_LOG` (default `info`) - log filter. Logs are JSON.
//!
//! Upstreams: `https://api.anthropic.com` for the Anthropic API (`/v1/`), and
//! for `/r/<host>/` the https hosts allowlisted by the grant of each proxied
//! secret. Prometheus metrics are served on `GET /metrics` of the same port.

use std::env::var;
use std::process::ExitCode;
use std::time::Duration;

use metrics_exporter_prometheus::PrometheusBuilder;
use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

use ironflow_auth_proxy::{
    AuthProxyConfig, AuthProxyState, DATABASE_URL_ENV, MIN_ADMIN_KEY_LEN, registry_from_config,
    serve, spawn_purge,
};
use ironflow_core::auth_proxy::{ADMIN_KEY_ENV, DEFAULT_UPSTREAM};
use ironflow_store::crypto::KeyRing;

/// Environment variable holding the listen address.
const LISTEN_ENV: &str = "IRONFLOW_AUTH_PROXY_LISTEN";

/// Listen address used when [`LISTEN_ENV`] is unset.
const DEFAULT_LISTEN: &str = "0.0.0.0:8080";

/// How often expired tokens are dropped.
const PURGE_INTERVAL: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().json().with_env_filter(filter).init();

    let listen = var(LISTEN_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_LISTEN.to_string());

    // A key read from a mounted file often ends with a newline.
    let admin_key = var(ADMIN_KEY_ENV)
        .map(|key| key.trim().to_string())
        .unwrap_or_default();
    if admin_key.len() < MIN_ADMIN_KEY_LEN {
        error!("{ADMIN_KEY_ENV} must be set to at least {MIN_ADMIN_KEY_LEN} characters");
        return ExitCode::FAILURE;
    }

    // The URL can carry a password: it is never logged.
    let database_url = var(DATABASE_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let key_ring = match &database_url {
        Some(_) => match KeyRing::from_env() {
            Ok(ring) => ring,
            Err(e) => {
                error!(error = %e, "invalid encryption key configuration");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let backend = if database_url.is_some() {
        "postgres"
    } else {
        "memory"
    };
    let registry = match registry_from_config(database_url.as_deref(), key_ring).await {
        Ok(registry) => registry,
        Err(e) => {
            error!(error = %e, backend, "cannot build the token registry");
            return ExitCode::FAILURE;
        }
    };

    let metrics = match PrometheusBuilder::new().install_recorder() {
        Ok(handle) => handle,
        Err(e) => {
            error!(error = %e, "cannot install the Prometheus recorder");
            return ExitCode::FAILURE;
        }
    };
    let state = match AuthProxyState::with_registry(AuthProxyConfig::new(&admin_key), registry) {
        Ok(state) => state.with_metrics(metrics),
        Err(e) => {
            error!(error = %e, "cannot build the upstream HTTP client");
            return ExitCode::FAILURE;
        }
    };
    let listener = match TcpListener::bind(&listen).await {
        Ok(listener) => listener,
        Err(e) => {
            error!(listen = %listen, error = %e, "cannot bind the listen address");
            return ExitCode::FAILURE;
        }
    };

    info!(
        listen = %listen,
        upstream = DEFAULT_UPSTREAM,
        backend,
        "ironflow-auth-proxy listening"
    );
    let purge = spawn_purge(state.registry().clone(), PURGE_INTERVAL);
    let result = serve(listener, state).await;
    purge.abort();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!(error = %e, "server stopped");
            ExitCode::FAILURE
        }
    }
}
