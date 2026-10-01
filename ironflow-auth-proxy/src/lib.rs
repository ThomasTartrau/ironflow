//! # ironflow-auth-proxy
//!
//! HTTP service that keeps the Claude credential out of ironflow agent pods.
//!
//! The worker (`K8sEphemeralProvider::auth_proxy`) asks the admin API for an
//! opaque token bound to one run and one step, and hands only that token to
//! the pod. Claude Code sends it as `Authorization: Bearer` to this proxy,
//! which swaps it for the real credential and relays the request to
//! `api.anthropic.com`, streaming the answer back.
//!
//! One listener serves:
//!
//! * `GET /healthz` - liveness;
//! * `/admin/v1/...` - token issuance and revocation, behind
//!   `Authorization: Bearer <IRONFLOW_AUTH_PROXY_ADMIN_KEY>`;
//! * anything else - the relay: an unknown, expired or revoked token gets a
//!   401, a path outside `/v1/` or a request for another host a 403, a method
//!   other than GET/POST a 405.
//!
//! Tokens live in memory: run a single replica. Logs never contain a token or
//! a credential, only the short token id.
//!
//! # Examples
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState, serve, spawn_purge};
//! use tokio::net::TcpListener;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let state = AuthProxyState::new(AuthProxyConfig::new("0123456789abcdef0123456789abcdef"))?;
//! let purge = spawn_purge(state.registry().clone(), Duration::from_secs(60));
//! let listener = TcpListener::bind("0.0.0.0:8080").await?;
//! serve(listener, state).await?;
//! purge.abort();
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::future::pending;
use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{DefaultBodyLimit, Path, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::serve as serve_router;
use axum::{Json, Router};
use reqwest::redirect::Policy;
use reqwest::{Client, Error as ReqwestError};
use serde_json::{from_slice, json};
use tokio::net::TcpListener;
use tokio::signal::ctrl_c;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinHandle;
use tokio::{select, spawn, time};
use tracing::{info, warn};
use url::Url;

use ironflow_core::auth_proxy::{
    AuthProxyError, AuthProxyRegistry, DEFAULT_UPSTREAM, TokenRejection, TokenRequest,
    admin_key_matches, downstream_headers, error_body, extract_opaque_token, is_allowed_method,
    is_allowed_path, upstream_headers,
};

/// Shortest admin key accepted.
pub const MIN_ADMIN_KEY_LEN: usize = 32;

/// Default largest request body relayed: 32 MiB.
pub const DEFAULT_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Length of the token id prefix written to logs.
const SHORT_ID_LEN: usize = 12;

/// Connect timeout towards the upstream. There is no total timeout: an
/// agent turn streams for as long as the model answers.
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Configuration of the proxy. Its [`Debug`] output never shows the admin key.
///
/// # Examples
///
/// ```
/// use ironflow_auth_proxy::AuthProxyConfig;
///
/// let config = AuthProxyConfig::new("0123456789abcdef0123456789abcdef");
/// assert_eq!(config.upstream.as_str(), "https://api.anthropic.com/");
/// assert!(!format!("{config:?}").contains("0123456789abcdef"));
/// ```
#[derive(Clone)]
pub struct AuthProxyConfig {
    /// Where requests are relayed. Always `https://api.anthropic.com` in the binary.
    pub upstream: Url,
    /// Key protecting the admin API.
    pub admin_key: String,
    /// Largest request body relayed, in bytes.
    pub max_body_bytes: usize,
}

impl fmt::Debug for AuthProxyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthProxyConfig")
            .field("upstream", &self.upstream.as_str())
            .field("admin_key", &"<redacted>")
            .field("max_body_bytes", &self.max_body_bytes)
            .finish()
    }
}

impl AuthProxyConfig {
    /// Configuration relaying to [`DEFAULT_UPSTREAM`] with the given admin key.
    ///
    /// # Panics
    ///
    /// Panics if `admin_key` is shorter than [`MIN_ADMIN_KEY_LEN`].
    ///
    /// # Examples
    ///
    /// See [`AuthProxyConfig`].
    pub fn new(admin_key: &str) -> Self {
        assert!(
            admin_key.len() >= MIN_ADMIN_KEY_LEN,
            "the auth proxy admin key must be at least {MIN_ADMIN_KEY_LEN} characters"
        );
        Self {
            upstream: Url::parse(DEFAULT_UPSTREAM).expect("DEFAULT_UPSTREAM is a valid URL"),
            admin_key: admin_key.to_string(),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    /// Relay to another upstream. **For tests only**: the binary never calls
    /// it, so a deployed proxy only reaches `api.anthropic.com`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_auth_proxy::AuthProxyConfig;
    /// use url::Url;
    ///
    /// # fn example() -> Result<(), url::ParseError> {
    /// let config = AuthProxyConfig::new("0123456789abcdef0123456789abcdef")
    ///     .with_upstream(Url::parse("http://127.0.0.1:9000")?);
    /// assert_eq!(config.upstream.port(), Some(9000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_upstream(mut self, upstream: Url) -> Self {
        self.upstream = upstream;
        self
    }
}

/// Shared state of the proxy: the token registry, the configuration and the
/// upstream HTTP client. Cheap to clone.
///
/// # Examples
///
/// ```
/// use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState};
///
/// # fn example() -> Result<(), reqwest::Error> {
/// let state = AuthProxyState::new(AuthProxyConfig::new("0123456789abcdef0123456789abcdef"))?;
/// assert!(state.registry().is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AuthProxyState {
    registry: AuthProxyRegistry,
    config: Arc<AuthProxyConfig>,
    http: Client,
}

impl AuthProxyState {
    /// Build the state with an empty registry.
    ///
    /// The upstream client never follows redirects and only bounds the
    /// connection time.
    ///
    /// # Errors
    ///
    /// Returns the [`reqwest::Error`] raised when the HTTP client cannot be
    /// built (TLS backend initialisation).
    ///
    /// # Examples
    ///
    /// See [`AuthProxyState`].
    pub fn new(config: AuthProxyConfig) -> Result<Self, ReqwestError> {
        let http = Client::builder()
            .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
            .redirect(Policy::none())
            .build()?;
        Ok(Self {
            registry: AuthProxyRegistry::default(),
            config: Arc::new(config),
            http,
        })
    }

    /// The token registry.
    ///
    /// # Examples
    ///
    /// See [`AuthProxyState`].
    pub fn registry(&self) -> &AuthProxyRegistry {
        &self.registry
    }
}

/// The proxy router: health, admin API and relay.
///
/// # Examples
///
/// ```no_run
/// use axum::serve;
/// use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState, router};
/// use tokio::net::TcpListener;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let state = AuthProxyState::new(AuthProxyConfig::new("0123456789abcdef0123456789abcdef"))?;
/// let listener = TcpListener::bind("127.0.0.1:0").await?;
/// serve(listener, router(state)).await?;
/// # Ok(())
/// # }
/// ```
pub fn router(state: AuthProxyState) -> Router {
    let limit = state.config.max_body_bytes;
    Router::new()
        .route("/healthz", get(healthz))
        .route("/admin/v1/tokens", post(issue_token))
        .route("/admin/v1/tokens/{id}", delete(revoke_token))
        .route("/admin/v1/runs/{run_id}/tokens", delete(revoke_run))
        .fallback(relay)
        .layer(DefaultBodyLimit::max(limit))
        .with_state(state)
}

/// Serve the proxy on `listener` until ctrl-c or SIGTERM, letting in-flight
/// requests finish.
///
/// # Errors
///
/// Returns the I/O error that stopped the server.
///
/// # Examples
///
/// See the [crate documentation](crate).
pub async fn serve(listener: TcpListener, state: AuthProxyState) -> io::Result<()> {
    serve_router(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
}

/// Spawn a task dropping the expired grants of `registry` every `interval`.
///
/// Abort the returned handle to stop it.
///
/// # Panics
///
/// Panics if `interval` is zero, or when called outside a Tokio runtime.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
///
/// use ironflow_auth_proxy::spawn_purge;
/// use ironflow_core::auth_proxy::AuthProxyRegistry;
///
/// # async fn example() {
/// let purge = spawn_purge(AuthProxyRegistry::default(), Duration::from_secs(60));
/// purge.abort();
/// # }
/// ```
pub fn spawn_purge(registry: AuthProxyRegistry, interval: Duration) -> JoinHandle<()> {
    assert!(
        !interval.is_zero(),
        "purge interval must be greater than zero"
    );
    spawn(async move {
        let mut ticker = time::interval(interval);
        loop {
            ticker.tick().await;
            match now_unix() {
                Ok(now) => {
                    let purged = registry.purge_expired(now);
                    if purged > 0 {
                        info!(purged, "expired auth proxy tokens purged");
                    }
                }
                Err(e) => warn!(error = %e, "clock before the unix epoch; purge skipped"),
            }
        }
    })
}

async fn shutdown_signal() {
    let interrupt = async {
        if let Err(e) = ctrl_c().await {
            warn!(error = %e, "cannot listen for ctrl-c");
            pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(e) => {
                warn!(error = %e, "cannot listen for SIGTERM");
                pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = pending::<()>();
    select! {
        () = interrupt => {},
        () = terminate => {},
    }
    info!("shutting down");
}

fn now_unix() -> Result<u64, SystemTimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
}

fn short_id(id: &str) -> &str {
    id.get(..SHORT_ID_LEN).unwrap_or(id)
}

fn error_response(status: StatusCode, kind: &str, message: &str) -> Response {
    (status, Json(error_body(kind, message))).into_response()
}

fn clock_error(e: &SystemTimeError) -> Response {
    warn!(error = %e, "system clock is before the unix epoch");
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "api_error",
        "auth proxy clock error",
    )
}

/// Whether the request carries the admin key as a bearer token.
fn admin_authorized(state: &AuthProxyState, headers: &HeaderMap) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|key| admin_key_matches(&state.config.admin_key, key.trim()))
}

/// The answer to a pod presenting no valid token. Logs the reason, never the token.
fn invalid_token(reason: &str, path: &str) -> Response {
    warn!(reason, path = %path, "request with an invalid token rejected");
    error_response(
        StatusCode::UNAUTHORIZED,
        "authentication_error",
        "invalid or expired ironflow auth proxy token",
    )
}

fn admin_unauthorized() -> Response {
    warn!("admin request without a valid admin key");
    error_response(
        StatusCode::UNAUTHORIZED,
        "authentication_error",
        "invalid or missing admin key",
    )
}

async fn healthz() -> &'static str {
    "ok"
}

async fn issue_token(
    State(state): State<AuthProxyState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return admin_unauthorized();
    }
    // The body carries a credential: neither the parse error nor the body
    // is ever echoed or logged.
    let Ok(request) = from_slice::<TokenRequest>(&body) else {
        warn!("token request body rejected");
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "invalid token request body",
        );
    };
    let now = match now_unix() {
        Ok(now) => now,
        Err(e) => return clock_error(&e),
    };
    let run_id = request.run_id.clone();
    let step = request.step.clone();
    match state.registry.issue(request, now) {
        Ok(issued) => {
            info!(
                token = %issued.short_id(),
                run_id = %run_id,
                step = %step,
                "token issued"
            );
            (StatusCode::CREATED, Json(issued)).into_response()
        }
        Err(AuthProxyError::InvalidRequest(message)) => {
            warn!(run_id = %run_id, step = %step, reason = %message, "token request refused");
            error_response(StatusCode::BAD_REQUEST, "invalid_request_error", &message)
        }
        Err(e) => {
            warn!(error = %e, "token issuance failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "token issuance failed",
            )
        }
    }
}

async fn revoke_token(
    State(state): State<AuthProxyState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return admin_unauthorized();
    }
    if state.registry.revoke(&id) {
        info!(token = %short_id(&id), "token revoked");
        StatusCode::NO_CONTENT.into_response()
    } else {
        error_response(StatusCode::NOT_FOUND, "not_found_error", "unknown token")
    }
}

async fn revoke_run(
    State(state): State<AuthProxyState>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return admin_unauthorized();
    }
    let revoked = state.registry.revoke_run(&run_id);
    info!(run_id = %run_id, revoked, "run tokens revoked");
    (StatusCode::OK, Json(json!({ "revoked": revoked }))).into_response()
}

async fn relay(State(state): State<AuthProxyState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let path = parts.uri.path().to_string();

    // Absolute-form (`GET http://host/..`) and CONNECT ask the proxy to reach
    // another host.
    if parts.uri.authority().is_some() || method == Method::CONNECT {
        warn!(method = %method, "request for another host refused");
        return error_response(
            StatusCode::FORBIDDEN,
            "permission_error",
            "only api.anthropic.com is reachable through this proxy",
        );
    }

    let now = match now_unix() {
        Ok(now) => now,
        Err(e) => return clock_error(&e),
    };
    let Some(token) = extract_opaque_token(&parts.headers) else {
        return invalid_token("missing", &path);
    };
    let grant = match state.registry.resolve(&token, now) {
        Ok(grant) => grant,
        Err(TokenRejection::Unknown) => return invalid_token("unknown", &path),
        Err(TokenRejection::Expired) => return invalid_token("expired", &path),
    };
    let token = short_id(&grant.id);

    if !is_allowed_path(&path) {
        warn!(token = %token, path = %path, "path outside the API refused");
        return error_response(
            StatusCode::FORBIDDEN,
            "permission_error",
            "only the Anthropic API under /v1/ is reachable through this proxy",
        );
    }
    if !is_allowed_method(&method) {
        warn!(token = %token, method = %method, path = %path, "method refused");
        return error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "invalid_request_error",
            "only GET and POST are relayed",
        );
    }

    let bytes = match to_bytes(body, state.config.max_body_bytes).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(token = %token, error = %e, "request body rejected");
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                "request body too large or unreadable",
            );
        }
    };

    let mut url = state.config.upstream.clone();
    url.set_path(&path);
    url.set_query(parts.uri.query());
    let upstream = state
        .http
        .request(method.clone(), url)
        .headers(upstream_headers(&parts.headers, &grant.credential))
        .body(bytes)
        .send()
        .await;
    let upstream = match upstream {
        Ok(upstream) => upstream,
        Err(e) => {
            warn!(token = %token, error = %e.without_url(), "upstream request failed");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "api_error",
                "the Anthropic API is unreachable",
            );
        }
    };

    let status = upstream.status();
    info!(
        token = %token,
        run_id = %grant.run_id,
        step = %grant.step,
        method = %method,
        path = %path,
        status = status.as_u16(),
        "relayed"
    );
    let headers = downstream_headers(upstream.headers());
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
