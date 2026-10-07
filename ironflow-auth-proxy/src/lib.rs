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
//! The same mechanism keeps other secrets (a GitHub or GitLab token, an API
//! key) out of the pod: a grant can hold a proxied secret with a host
//! allowlist instead of the Claude credential. The pod calls
//! `/r/<host>/<path>` with its opaque token (as `Authorization: Bearer`,
//! `x-api-key`, `Private-Token` or the password of `Authorization: Basic`),
//! and the proxy relays to `https://<host>/<path>` with the real secret
//! injected the way the grant says (bearer, `Private-Token`, `x-api-key`, a
//! named header or Basic). A host outside the allowlist (exact names or a
//! leading `*.`, no port, no IP) gets a 403, so does a Claude token on `/r/`
//! and a secret token on the Anthropic API. Redirects are returned to the
//! pod, never followed.
//!
//! One listener serves:
//!
//! * `GET /healthz` - liveness;
//! * `GET /metrics` - Prometheus metrics, when a recorder handle is given
//!   ([`AuthProxyState::with_metrics`]): [`REQUESTS_TOTAL`] counts the `/r/`
//!   requests by `secret` name and `result`;
//! * `/admin/v1/...` - token issuance and revocation, behind
//!   `Authorization: Bearer <IRONFLOW_AUTH_PROXY_ADMIN_KEY>`;
//! * `/r/<host>/<path>` - the relay of proxied secrets: an unknown, expired
//!   or revoked token gets a 401, a host outside the allowlist or a path with
//!   `..`, `//` or percent-encoding a 403, CONNECT, TRACE or OPTIONS a 405.
//!   Each request logs one `secret relay` event with its `result`
//!   (`relayed`, `forbidden_host`, `forbidden_path`, `forbidden_method`,
//!   `unknown_token`, `expired`, `revoked`, `upstream_error`,
//!   `unavailable`);
//! * anything else - the Anthropic relay: an unknown, expired or revoked
//!   token gets a 401, a path outside `/v1/` or a request for another host a
//!   403, a method other than GET/POST a 405.
//!
//! Grants live in a registry with two backends:
//!
//! * in memory (default): a single replica, tokens lost on restart;
//! * PostgreSQL, when [`DATABASE_URL_ENV`] is set ([`registry_from_config`]):
//!   shared by several replicas and surviving restarts. Only the token
//!   SHA-256 is stored, never the token, and the credential is AES-256-GCM
//!   encrypted at rest with the `IRONFLOW_SECRET_KEYS` key ring.
//!
//! Logs never contain a token, a credential, a secret, a header or a query
//! string, only the short token id.
//!
//! # Examples
//!
//! ```no_run
//! use std::env::var;
//! use std::time::Duration;
//!
//! use ironflow_auth_proxy::{
//!     AuthProxyConfig, AuthProxyState, DATABASE_URL_ENV, registry_from_config, serve, spawn_purge,
//! };
//! use ironflow_store::crypto::KeyRing;
//! use tokio::net::TcpListener;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let database_url = var(DATABASE_URL_ENV).ok();
//! let registry = registry_from_config(database_url.as_deref(), KeyRing::from_env()?).await?;
//! let config = AuthProxyConfig::new("0123456789abcdef0123456789abcdef");
//! let state = AuthProxyState::with_registry(config, registry)?;
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
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::serve as serve_router;
use axum::{Json, Router};
use metrics::counter;
use metrics_exporter_prometheus::PrometheusHandle;
use reqwest::redirect::Policy;
use reqwest::{Client, Error as ReqwestError};
use serde_json::{from_slice, json};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::signal::ctrl_c;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinHandle;
use tokio::{select, spawn, time};
use tracing::{info, warn};
use url::Url;

use ironflow_core::auth_proxy::{
    AuthProxyError, AuthProxyRegistry, DEFAULT_UPSTREAM, Grant, GrantCredential, RELAY_PREFIX,
    TokenRejection, TokenRequest, admin_key_matches, downstream_headers, error_body,
    extract_opaque_token, is_allowed_method, is_allowed_path, is_relay_method, is_relay_path,
    is_valid_request_host, secret_upstream_headers, upstream_headers,
};
use ironflow_store::crypto::{CryptoError, KeyRing};
use ironflow_store::error::StoreError;
use ironflow_store::postgres::PostgresStore;

/// Environment variable holding the PostgreSQL URL of the shared token
/// registry. Unset (or empty), tokens live in memory.
pub const DATABASE_URL_ENV: &str = "IRONFLOW_AUTH_PROXY_DATABASE_URL";

/// Shortest admin key accepted.
pub const MIN_ADMIN_KEY_LEN: usize = 32;

/// Default largest request body relayed: 32 MiB.
pub const DEFAULT_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Prometheus counter of the requests to the `/r/` relay, labelled by
/// `secret` (its name, empty for an unknown token) and `result`.
pub const REQUESTS_TOTAL: &str = "ironflow_auth_proxy_requests_total";

/// Content type of the Prometheus text exposition format.
const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4";

/// Base of the URL of a `/r/` request, whose host is then replaced by the
/// requested one.
const SECRET_UPSTREAM_BASE: &str = "https://localhost";

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
/// assert!(config.secret_upstream.is_none());
/// assert!(!format!("{config:?}").contains("0123456789abcdef"));
/// ```
#[derive(Clone)]
pub struct AuthProxyConfig {
    /// Where requests are relayed. Always `https://api.anthropic.com` in the binary.
    pub upstream: Url,
    /// Where `/r/<host>/` requests are relayed instead of `https://<host>`.
    /// Always `None` in the binary.
    pub secret_upstream: Option<Url>,
    /// Key protecting the admin API.
    pub admin_key: String,
    /// Largest request body relayed, in bytes.
    pub max_body_bytes: usize,
}

impl fmt::Debug for AuthProxyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthProxyConfig")
            .field("upstream", &self.upstream.as_str())
            .field(
                "secret_upstream",
                &self.secret_upstream.as_ref().map(Url::as_str),
            )
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
            secret_upstream: None,
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

    /// Relay every `/r/<host>/` request to `upstream` instead of
    /// `https://<host>`. The host allowlist is still checked on `<host>`.
    /// **For tests only**: the binary never calls it, so a deployed proxy
    /// only reaches the allowlisted hosts, over https.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_auth_proxy::AuthProxyConfig;
    /// use url::Url;
    ///
    /// # fn example() -> Result<(), url::ParseError> {
    /// let config = AuthProxyConfig::new("0123456789abcdef0123456789abcdef")
    ///     .with_secret_upstream(Url::parse("http://127.0.0.1:9001")?);
    /// assert_eq!(config.secret_upstream.and_then(|url| url.port()), Some(9001));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_secret_upstream(mut self, upstream: Url) -> Self {
        self.secret_upstream = Some(upstream);
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
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let state = AuthProxyState::new(AuthProxyConfig::new("0123456789abcdef0123456789abcdef"))?;
/// assert!(state.registry().is_empty().await?);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AuthProxyState {
    registry: AuthProxyRegistry,
    config: Arc<AuthProxyConfig>,
    http: Client,
    metrics: Option<PrometheusHandle>,
}

impl AuthProxyState {
    /// Build the state with an empty in-memory registry.
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
        Self::with_registry(config, AuthProxyRegistry::default())
    }

    /// Build the state over `registry`, for instance the shared PostgreSQL
    /// registry returned by [`registry_from_config`]. Proxies built over
    /// registries sharing a backend serve the same tokens.
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
    /// ```
    /// use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState};
    /// use ironflow_core::auth_proxy::AuthProxyRegistry;
    ///
    /// # fn example() -> Result<(), reqwest::Error> {
    /// let registry = AuthProxyRegistry::default();
    /// let config = AuthProxyConfig::new("0123456789abcdef0123456789abcdef");
    /// let a = AuthProxyState::with_registry(config.clone(), registry.clone())?;
    /// let b = AuthProxyState::with_registry(config, registry)?;
    /// # let _ = (a, b);
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_registry(
        config: AuthProxyConfig,
        registry: AuthProxyRegistry,
    ) -> Result<Self, ReqwestError> {
        let http = Client::builder()
            .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
            .redirect(Policy::none())
            .build()?;
        Ok(Self {
            registry,
            config: Arc::new(config),
            http,
            metrics: None,
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

    /// Serve `GET /metrics` from `handle`, the handle of the installed
    /// Prometheus recorder. Without it, `/metrics` answers 404.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState};
    /// use metrics_exporter_prometheus::PrometheusBuilder;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handle = PrometheusBuilder::new().install_recorder()?;
    /// let state = AuthProxyState::new(AuthProxyConfig::new("0123456789abcdef0123456789abcdef"))?
    ///     .with_metrics(handle);
    /// # let _ = state;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_metrics(mut self, handle: PrometheusHandle) -> Self {
        self.metrics = Some(handle);
        self
    }
}

/// Why the token registry could not be built. No variant carries the
/// database URL, a key or a credential.
///
/// # Examples
///
/// ```
/// use ironflow_auth_proxy::RegistryConfigError;
///
/// assert!(RegistryConfigError::MissingKeyRing.to_string().contains("IRONFLOW_SECRET_KEYS"));
/// ```
#[derive(Debug, Error)]
pub enum RegistryConfigError {
    /// A database is configured but no key to encrypt the credentials with.
    #[error(
        "{DATABASE_URL_ENV} is set but no encryption key is configured: set IRONFLOW_SECRET_KEYS (or IRONFLOW_SECRET_KEY)"
    )]
    MissingKeyRing,
    /// The encryption key configuration is invalid.
    #[error("invalid encryption key: {0}")]
    Crypto(#[from] CryptoError),
    /// The database cannot be opened or migrated.
    #[error("cannot open the token registry database: {0}")]
    Store(#[from] StoreError),
}

/// Build the token registry: in memory when `database_url` is `None` or
/// blank, otherwise shared in PostgreSQL, the credentials encrypted with
/// `key_ring`. Opening the database runs the store migrations.
///
/// # Errors
///
/// Returns [`RegistryConfigError::MissingKeyRing`] when a database is given
/// without a key ring (checked before any connection), and
/// [`RegistryConfigError::Store`] when the database cannot be reached or
/// migrated.
///
/// # Examples
///
/// ```no_run
/// use ironflow_auth_proxy::registry_from_config;
/// use ironflow_store::crypto::KeyRing;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let memory = registry_from_config(None, None).await?;
/// assert!(memory.is_empty().await?);
///
/// let ring = KeyRing::from_spec(&format!("1:{}", "aa".repeat(32)), None)?;
/// let shared = registry_from_config(Some("postgres://localhost/ironflow"), Some(ring)).await?;
/// # let _ = shared;
/// # Ok(())
/// # }
/// ```
pub async fn registry_from_config(
    database_url: Option<&str>,
    key_ring: Option<KeyRing>,
) -> Result<AuthProxyRegistry, RegistryConfigError> {
    let Some(url) = database_url.map(str::trim).filter(|url| !url.is_empty()) else {
        return Ok(AuthProxyRegistry::default());
    };
    let ring = key_ring.ok_or(RegistryConfigError::MissingKeyRing)?;
    let mut store = PostgresStore::new(url).await?;
    store.set_key_ring(ring);
    Ok(AuthProxyRegistry::with_backend(Arc::new(store)))
}

/// The proxy router: health, metrics, admin API and relays.
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
        .route("/metrics", get(serve_metrics))
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
                // Every replica purges: the deletes are idempotent.
                Ok(now) => match registry.purge_expired(now).await {
                    Ok(purged) if purged > 0 => info!(purged, "expired auth proxy tokens purged"),
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "expired auth proxy tokens purge failed"),
                },
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
    unauthorized()
}

/// The 401 answer to an unknown, expired or revoked token.
fn unauthorized() -> Response {
    error_response(
        StatusCode::UNAUTHORIZED,
        "authentication_error",
        "invalid or expired ironflow auth proxy token",
    )
}

/// The answer when the registry backend fails. A 503 is retryable, and does
/// not make a valid token look revoked as a 401 would.
fn registry_unavailable(message: &str) -> Response {
    error_response(StatusCode::SERVICE_UNAVAILABLE, "api_error", message)
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

async fn serve_metrics(State(state): State<AuthProxyState>) -> Response {
    match &state.metrics {
        Some(handle) => ([(CONTENT_TYPE, METRICS_CONTENT_TYPE)], handle.render()).into_response(),
        None => error_response(
            StatusCode::NOT_FOUND,
            "not_found_error",
            "metrics are disabled",
        ),
    }
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
    // The name only: the value never reaches a log.
    let secret = match &request.credential {
        GrantCredential::Secret(secret) => Some(secret.name().to_string()),
        GrantCredential::Claude(_) => None,
    };
    match state.registry.issue(request, now).await {
        Ok(issued) => {
            info!(
                token = %issued.short_id(),
                run_id = %run_id,
                step = %step,
                secret = secret.as_deref(),
                "token issued"
            );
            (StatusCode::CREATED, Json(issued)).into_response()
        }
        Err(AuthProxyError::InvalidRequest(message)) => {
            warn!(run_id = %run_id, step = %step, reason = %message, "token request refused");
            error_response(StatusCode::BAD_REQUEST, "invalid_request_error", &message)
        }
        Err(AuthProxyError::Backend(e)) => {
            warn!(run_id = %run_id, step = %step, error = %e, "token registry unavailable");
            registry_unavailable("token registry unavailable")
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
    match state.registry.revoke(&id).await {
        Ok(true) => {
            info!(token = %short_id(&id), "token revoked");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found_error", "unknown token"),
        Err(e) => {
            warn!(token = %short_id(&id), error = %e, "token registry unavailable");
            registry_unavailable("token registry unavailable")
        }
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
    match state.registry.revoke_run(&run_id).await {
        Ok(revoked) => {
            info!(run_id = %run_id, revoked, "run tokens revoked");
            (StatusCode::OK, Json(json!({ "revoked": revoked }))).into_response()
        }
        Err(e) => {
            warn!(run_id = %run_id, error = %e, "token registry unavailable");
            registry_unavailable("token registry unavailable")
        }
    }
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
    if let Some(rest) = path
        .strip_prefix(RELAY_PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        let query = parts.uri.query();
        return relay_secret(&state, method, &parts.headers, query, body, rest).await;
    }

    let now = match now_unix() {
        Ok(now) => now,
        Err(e) => return clock_error(&e),
    };
    let Some(token) = extract_opaque_token(&parts.headers) else {
        return invalid_token("missing", &path);
    };
    let grant = match state.registry.resolve(&token, now).await {
        Ok(grant) => grant,
        Err(TokenRejection::Unknown) => return invalid_token("unknown", &path),
        Err(TokenRejection::Expired) => return invalid_token("expired", &path),
        Err(TokenRejection::Revoked) => return invalid_token("revoked", &path),
        Err(TokenRejection::Unavailable(e)) => {
            warn!(error = %e, path = %path, "token registry unavailable");
            return registry_unavailable("auth proxy token registry unavailable");
        }
    };
    let token = short_id(&grant.id);
    let GrantCredential::Claude(credential) = &grant.credential else {
        warn!(token = %token, path = %path, "secret token on the Anthropic API refused");
        return error_response(
            StatusCode::FORBIDDEN,
            "permission_error",
            "this token only reaches its allowlisted hosts under /r/",
        );
    };

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
        .headers(upstream_headers(&parts.headers, credential))
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

/// Outcome of a `/r/` request: the `result` label of [`REQUESTS_TOTAL`]
/// and the `result` field of its log event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RelayResult {
    Relayed,
    ForbiddenHost,
    ForbiddenPath,
    ForbiddenMethod,
    UnknownToken,
    Expired,
    Revoked,
    UpstreamError,
    Unavailable,
}

impl RelayResult {
    fn as_str(self) -> &'static str {
        match self {
            Self::Relayed => "relayed",
            Self::ForbiddenHost => "forbidden_host",
            Self::ForbiddenPath => "forbidden_path",
            Self::ForbiddenMethod => "forbidden_method",
            Self::UnknownToken => "unknown_token",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::UpstreamError => "upstream_error",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Count a `/r/` request and log it. `secret` is empty when the token is
/// unknown. Never given the token, the secret value, a header or the query.
fn record(
    result: RelayResult,
    host: &str,
    secret: &str,
    grant: Option<&Grant>,
    upstream_status: Option<u16>,
) {
    counter!(REQUESTS_TOTAL, "secret" => secret.to_string(), "result" => result.as_str())
        .increment(1);
    let token = grant.map(|grant| short_id(&grant.id));
    let run_id = grant.map(|grant| grant.run_id.as_str());
    let step = grant.map(|grant| grant.step.as_str());
    if result == RelayResult::Relayed {
        info!(
            host,
            secret,
            run_id,
            step,
            token,
            result = result.as_str(),
            upstream_status,
            "secret relay"
        );
    } else {
        warn!(
            host,
            secret,
            run_id,
            step,
            token,
            result = result.as_str(),
            upstream_status,
            "secret relay"
        );
    }
}

/// Relay `/r/<host>/<path>` (`rest` is `<host>/<path>`) to `https://<host>`
/// with the proxied secret of the token's grant.
async fn relay_secret(
    state: &AuthProxyState,
    method: Method,
    headers: &HeaderMap,
    query: Option<&str>,
    body: Body,
    rest: &str,
) -> Response {
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host.to_ascii_lowercase(), format!("/{path}")),
        None => (rest.to_ascii_lowercase(), "/".to_string()),
    };
    // Never echo a host that failed validation into a log.
    let logged_host = if is_valid_request_host(&host) {
        host.as_str()
    } else {
        "<invalid>"
    };

    let now = match now_unix() {
        Ok(now) => now,
        Err(e) => return clock_error(&e),
    };
    let Some(token) = extract_opaque_token(headers) else {
        record(RelayResult::UnknownToken, logged_host, "", None, None);
        return unauthorized();
    };
    let grant = match state.registry.resolve(&token, now).await {
        Ok(grant) => grant,
        Err(rejection) => {
            let result = match rejection {
                TokenRejection::Unknown => RelayResult::UnknownToken,
                TokenRejection::Expired => RelayResult::Expired,
                TokenRejection::Revoked => RelayResult::Revoked,
                TokenRejection::Unavailable(e) => {
                    warn!(error = %e, "token registry unavailable");
                    record(RelayResult::Unavailable, logged_host, "", None, None);
                    return registry_unavailable("auth proxy token registry unavailable");
                }
            };
            record(result, logged_host, "", None, None);
            return unauthorized();
        }
    };

    let GrantCredential::Secret(secret) = &grant.credential else {
        record(
            RelayResult::ForbiddenHost,
            logged_host,
            "",
            Some(&grant),
            None,
        );
        return error_response(
            StatusCode::FORBIDDEN,
            "permission_error",
            "a Claude token only reaches the Anthropic API",
        );
    };
    let name = secret.name();
    let refuse = |result| record(result, logged_host, name, Some(&grant), None);
    let forbidden_host = || {
        refuse(RelayResult::ForbiddenHost);
        error_response(
            StatusCode::FORBIDDEN,
            "permission_error",
            "host not allowed for this secret",
        )
    };
    if !secret.allows_host(&host) {
        return forbidden_host();
    }
    if !is_relay_path(&path) {
        refuse(RelayResult::ForbiddenPath);
        return error_response(
            StatusCode::FORBIDDEN,
            "permission_error",
            "path not allowed: no '..', '//' or percent-encoding",
        );
    }
    if !is_relay_method(&method) {
        refuse(RelayResult::ForbiddenMethod);
        return error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "invalid_request_error",
            "only GET, HEAD, POST, PUT, PATCH and DELETE are relayed",
        );
    }

    let bytes = match to_bytes(body, state.config.max_body_bytes).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(
                token = %short_id(&grant.id),
                secret = name,
                error = %e,
                "request body rejected"
            );
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                "request body too large or unreadable",
            );
        }
    };

    let mut url = match &state.config.secret_upstream {
        Some(upstream) => upstream.clone(),
        None => {
            // `host` passed the allowlist, so it is a DNS name: this only
            // fails if the URL parser disagrees, and then refuses it.
            let url = Url::parse(SECRET_UPSTREAM_BASE).ok().and_then(|mut url| {
                url.set_host(Some(&host)).ok()?;
                Some(url)
            });
            let Some(url) = url else {
                return forbidden_host();
            };
            url
        }
    };
    url.set_path(&path);
    url.set_query(query);
    let upstream = state
        .http
        .request(method, url)
        .headers(secret_upstream_headers(headers, secret))
        .body(bytes)
        .send()
        .await;
    let upstream = match upstream {
        Ok(upstream) => upstream,
        Err(e) => {
            warn!(
                token = %short_id(&grant.id),
                secret = name,
                error = %e.without_url(),
                "upstream request failed"
            );
            refuse(RelayResult::UpstreamError);
            return error_response(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "upstream unreachable",
            );
        }
    };

    let status = upstream.status();
    record(
        RelayResult::Relayed,
        logged_host,
        name,
        Some(&grant),
        Some(status.as_u16()),
    );
    let headers = downstream_headers(upstream.headers());
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_ring() -> KeyRing {
        KeyRing::from_spec(&format!("1:{}", "aa".repeat(32)), None).unwrap()
    }

    #[tokio::test]
    async fn registry_from_config_defaults_to_memory() {
        let registry = registry_from_config(None, None).await.unwrap();
        assert!(registry.is_empty().await.unwrap());

        let registry = registry_from_config(Some(""), None).await.unwrap();
        assert!(registry.is_empty().await.unwrap());

        let registry = registry_from_config(Some("  \n"), Some(key_ring()))
            .await
            .unwrap();
        assert!(registry.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn registry_from_config_requires_key_ring_with_database() {
        let result = registry_from_config(Some("postgres://localhost/x"), None).await;
        assert!(
            matches!(result, Err(RegistryConfigError::MissingKeyRing)),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn registry_from_config_rejects_invalid_database_url() {
        let result = registry_from_config(Some("not a url"), Some(key_ring())).await;
        assert!(
            matches!(result, Err(RegistryConfigError::Store(_))),
            "{result:?}"
        );
    }
    #[test]
    fn relay_results_have_stable_labels() {
        let labels: Vec<&str> = [
            RelayResult::Relayed,
            RelayResult::ForbiddenHost,
            RelayResult::ForbiddenPath,
            RelayResult::ForbiddenMethod,
            RelayResult::UnknownToken,
            RelayResult::Expired,
            RelayResult::Revoked,
            RelayResult::UpstreamError,
            RelayResult::Unavailable,
        ]
        .into_iter()
        .map(RelayResult::as_str)
        .collect();
        assert_eq!(
            labels,
            vec![
                "relayed",
                "forbidden_host",
                "forbidden_path",
                "forbidden_method",
                "unknown_token",
                "expired",
                "revoked",
                "upstream_error",
                "unavailable",
            ]
        );
    }

    #[test]
    fn config_debug_shows_secret_upstream_not_admin_key() {
        let config = AuthProxyConfig::new("0123456789abcdef0123456789abcdef")
            .with_secret_upstream(Url::parse("http://127.0.0.1:9001").unwrap());
        let debug = format!("{config:?}");
        assert!(debug.contains("http://127.0.0.1:9001/"), "{debug}");
        assert!(!debug.contains("0123456789abcdef"), "{debug}");
    }
}
