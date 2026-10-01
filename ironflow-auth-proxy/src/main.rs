//! `ironflow-auth-proxy` binary.
//!
//! Environment:
//!
//! * `IRONFLOW_AUTH_PROXY_ADMIN_KEY` (required, at least 32 characters) - key
//!   of the admin API, shared with the worker.
//! * `IRONFLOW_AUTH_PROXY_LISTEN` (default `0.0.0.0:8080`) - listen address.
//! * `RUST_LOG` (default `info`) - log filter. Logs are JSON.
//!
//! The upstream is always `https://api.anthropic.com`.

use std::env::var;
use std::process::ExitCode;
use std::time::Duration;

use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState, MIN_ADMIN_KEY_LEN, serve, spawn_purge};
use ironflow_core::auth_proxy::{ADMIN_KEY_ENV, DEFAULT_UPSTREAM};

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

    let state = match AuthProxyState::new(AuthProxyConfig::new(&admin_key)) {
        Ok(state) => state,
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

    info!(listen = %listen, upstream = DEFAULT_UPSTREAM, "ironflow-auth-proxy listening");
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
