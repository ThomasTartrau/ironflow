//! The runtime server builder and HTTP serving logic.
//!
//! [`Runtime`] is the central entry-point for configuring and launching an
//! ironflow daemon. It uses a builder pattern to register webhook routes and
//! cron jobs, then starts an [Axum](https://docs.rs/axum) HTTP server with
//! graceful shutdown on `Ctrl+C`.
//!
//! Webhook handlers are executed in the background via [`tokio::spawn`], so
//! the HTTP endpoint responds with **202 Accepted** immediately while the
//! workflow runs asynchronously.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware;
use axum::routing::{get, post};
use serde_json::{Value, from_slice};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info, warn};

use crate::cron::CronJob;
use crate::error::RuntimeError;
use crate::webhook::{WebhookAuth, extract_delivery_id};

/// Default maximum body size for webhook payloads (2 MiB).
const DEFAULT_MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Default maximum number of concurrently running webhook handlers.
const DEFAULT_MAX_CONCURRENT_HANDLERS: usize = 64;

type WebhookHandler =
    Arc<dyn Fn(WebhookContext) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
type ShutdownSignal = Pin<Box<dyn Future<Output = ()> + Send>>;

/// What a webhook handler registered via [`Runtime::webhook_with_context`] receives.
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::prelude::*;
///
/// let runtime = Runtime::new().webhook_with_context(
///     "/hooks/github",
///     WebhookAuth::github("secret"),
///     |ctx: WebhookContext| async move {
///         println!("{} from {:?}", ctx.payload, ctx.delivery_id);
///     },
/// );
/// ```
#[derive(Debug, Clone)]
pub struct WebhookContext {
    /// The parsed JSON body.
    pub payload: Value,
    /// Provider delivery identifier, already prefixed (`github:…`, `gitlab:…`).
    ///
    /// `None` when the provider sent no known delivery header. Suitable as-is
    /// for the `Idempotency-Key` header of `POST /api/v1/runs`.
    pub delivery_id: Option<String>,
}

/// Metric name constants for the runtime (webhook + cron).
#[cfg(feature = "prometheus")]
mod metric_names {
    pub const WEBHOOK_RECEIVED_TOTAL: &str = "ironflow_webhook_received_total";
    pub const CRON_RUNS_TOTAL: &str = "ironflow_cron_runs_total";

    pub const AUTH_REJECTED: &str = "rejected";
    pub const AUTH_ACCEPTED: &str = "accepted";
    pub const AUTH_INVALID_BODY: &str = "invalid_body";
}

struct WebhookRoute {
    path: String,
    auth: WebhookAuth,
    handler: WebhookHandler,
}

/// The ironflow runtime server builder.
///
/// `Runtime` uses a builder pattern: create one with [`Runtime::new`], register
/// webhook routes with [`Runtime::webhook`] and cron jobs with
/// [`Runtime::cron`], then call [`Runtime::serve`] to start both the HTTP
/// server and the cron scheduler, or [`Runtime::run_crons`] to run only the
/// cron scheduler without an HTTP listener.
///
/// # Built-in endpoints
///
/// | Method | Path | Description |
/// |--------|------|-------------|
/// | `GET`  | `/health` | Returns `200 OK` with body `"ok"`. Useful for load-balancer health checks. |
/// | `POST` | *user-defined* | Webhook endpoints registered via [`Runtime::webhook`]. |
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::prelude::*;
///
/// #[tokio::main]
/// async fn main() -> Result<(), ironflow_runtime::error::RuntimeError> {
///     Runtime::new()
///         .webhook("/hooks/deploy", WebhookAuth::github("secret"), |payload| async move {
///             println!("deploy triggered: {payload}");
///         })
///         .cron("0 0 * * * *", "hourly-sync", || async {
///             println!("syncing...");
///         })
///         .serve("0.0.0.0:3000")
///         .await?;
///
///     Ok(())
/// }
/// ```
pub struct Runtime {
    webhooks: Vec<WebhookRoute>,
    crons: Vec<CronJob>,
    max_body_size: usize,
    max_concurrent_handlers: usize,
    custom_shutdown: Option<ShutdownSignal>,
}

impl Runtime {
    /// Creates a new, empty `Runtime` with no webhooks or cron jobs.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::runtime::Runtime;
    ///
    /// let runtime = Runtime::new();
    /// ```
    pub fn new() -> Self {
        Self {
            webhooks: Vec::new(),
            crons: Vec::new(),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            max_concurrent_handlers: DEFAULT_MAX_CONCURRENT_HANDLERS,
            custom_shutdown: None,
        }
    }

    /// Set the maximum allowed body size for webhook payloads.
    ///
    /// Requests exceeding this limit are rejected by axum before reaching the
    /// handler. Defaults to 2 MiB.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::runtime::Runtime;
    ///
    /// let runtime = Runtime::new().max_body_size(512 * 1024); // 512 KiB
    /// ```
    pub fn max_body_size(mut self, bytes: usize) -> Self {
        self.max_body_size = bytes;
        self
    }

    /// Set the maximum number of concurrently running webhook handlers.
    ///
    /// When the limit is reached, new webhook requests still receive
    /// **202 Accepted** but their handlers are queued until a slot is
    /// available. Defaults to 64.
    ///
    /// # Panics
    ///
    /// Panics if `limit` is `0`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::runtime::Runtime;
    ///
    /// let runtime = Runtime::new().max_concurrent_handlers(16);
    /// ```
    pub fn max_concurrent_handlers(mut self, limit: usize) -> Self {
        assert!(limit > 0, "max_concurrent_handlers must be greater than 0");
        self.max_concurrent_handlers = limit;
        self
    }

    /// Override the default shutdown signal (`Ctrl+C` / `SIGTERM`).
    ///
    /// By default, [`Runtime::serve`] and [`Runtime::run_crons`] block until
    /// the process receives `Ctrl+C` or `SIGTERM`. Use this method to provide
    /// a custom future that resolves when the runtime should shut down.
    ///
    /// This is useful in tests where you want to trigger a clean shutdown
    /// (including `scheduler.shutdown()`) without relying on OS signals.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::runtime::Runtime;
    /// use tokio::sync::oneshot;
    ///
    /// # async fn example() -> Result<(), ironflow_runtime::error::RuntimeError> {
    /// let (tx, rx) = oneshot::channel::<()>();
    ///
    /// let rt = Runtime::new()
    ///     .with_shutdown(async { let _ = rx.await; })
    ///     .cron("0 */5 * * * *", "check", || async {});
    ///
    /// // In another task: tx.send(()) to trigger shutdown.
    /// rt.run_crons().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_shutdown<F>(mut self, signal: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.custom_shutdown = Some(Box::pin(signal));
        self
    }

    /// Registers a webhook route.
    ///
    /// The handler receives the parsed JSON body as a [`serde_json::Value`].
    /// When a request arrives, the server verifies authentication using `auth`,
    /// then spawns the handler in the background and immediately returns
    /// **202 Accepted** to the caller.
    ///
    /// # Arguments
    ///
    /// * `path` - The URL path to listen on (e.g. `"/hooks/github"`).
    /// * `auth` - The [`WebhookAuth`] strategy for this endpoint.
    /// * `handler` - An async function receiving the JSON payload.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::prelude::*;
    ///
    /// let runtime = Runtime::new()
    ///     .webhook("/hooks/github", WebhookAuth::github("secret"), |payload| async move {
    ///         println!("payload: {payload}");
    ///     });
    /// ```
    pub fn webhook<F, Fut>(self, path: &str, auth: WebhookAuth, handler: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.webhook_with_context(path, auth, move |ctx| handler(ctx.payload))
    }

    /// Registers a webhook route whose handler also sees the delivery identifier.
    ///
    /// Identical to [`Runtime::webhook`], except the handler receives a
    /// [`WebhookContext`] instead of the bare payload. Use this when the workflow
    /// should be enqueued idempotently: replaying the same provider delivery then
    /// resolves to the run it already created rather than a duplicate.
    ///
    /// # Arguments
    ///
    /// * `path` - The URL path to listen on (e.g. `"/hooks/github"`).
    /// * `auth` - The [`WebhookAuth`] strategy for this endpoint.
    /// * `handler` - An async function receiving the [`WebhookContext`].
    ///
    /// # Panics
    ///
    /// Panics if `path` does not start with `/`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::prelude::*;
    ///
    /// let runtime = Runtime::new().webhook_with_context(
    ///     "/hooks/github",
    ///     WebhookAuth::github("secret"),
    ///     |ctx| async move {
    ///         // `ctx.delivery_id` is `Some("github:<X-GitHub-Delivery>")` when
    ///         // the provider stamped the request.
    ///         println!("delivery: {:?}", ctx.delivery_id);
    ///     },
    /// );
    /// ```
    pub fn webhook_with_context<F, Fut>(mut self, path: &str, auth: WebhookAuth, handler: F) -> Self
    where
        F: Fn(WebhookContext) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        assert!(
            path.starts_with('/'),
            "webhook path must start with '/', got: {path}"
        );
        if matches!(auth, WebhookAuth::None) {
            warn!(path = %path, "webhook registered with WebhookAuth::None - all requests will be accepted without authentication");
        }
        let handler: WebhookHandler = Arc::new(move |ctx| {
            let handler = handler.clone();
            Box::pin(async move { handler(ctx).await })
        });
        self.webhooks.push(WebhookRoute {
            path: path.to_string(),
            auth,
            handler,
        });
        self
    }

    /// Registers a cron job.
    ///
    /// The `schedule` uses a **6-field cron expression** (seconds granularity):
    /// `sec min hour day-of-month month day-of-week`.
    ///
    /// # Arguments
    ///
    /// * `schedule` - A 6-field cron expression, e.g. `"0 */5 * * * *"` for every 5 minutes.
    /// * `name` - A human-readable name for logging.
    /// * `handler` - An async function to execute on each tick.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::prelude::*;
    ///
    /// let runtime = Runtime::new()
    ///     .cron("0 0 * * * *", "hourly-cleanup", || async {
    ///         println!("cleaning up...");
    ///     });
    /// ```
    pub fn cron<F, Fut>(mut self, schedule: &str, name: &str, handler: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler_fn: Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync> =
            Box::new(move || Box::pin(handler()));
        self.crons.push(CronJob {
            schedule: schedule.to_string(),
            name: name.to_string(),
            handler: handler_fn,
        });
        self
    }

    /// Build the axum [`Router`] from the registered webhooks.
    ///
    /// This is separated from [`Runtime::serve`] so the router can be tested
    /// independently (e.g. with `tower::ServiceExt::oneshot` or by
    /// binding to a random port in integration tests).
    fn build_router(
        webhooks: Vec<WebhookRoute>,
        handler_tracker: Arc<HandlerTracker>,
        max_body_size: usize,
        #[cfg(feature = "prometheus")] prom_handle: Option<
            metrics_exporter_prometheus::PrometheusHandle,
        >,
    ) -> Router {
        let mut router = Router::new();

        for webhook in webhooks {
            let auth = Arc::new(webhook.auth);
            let handler = webhook.handler;
            let path = webhook.path.clone();

            let name: Arc<str> = Arc::from(path.as_str());
            let route_state = WebhookState {
                auth,
                handler,
                name,
                tracker: handler_tracker.clone(),
            };

            router = router.route(&path, post(webhook_handler).with_state(route_state));
            info!(path = %path, "registered webhook");
        }

        router = router.route("/health", get(|| async { "ok" }));

        #[cfg(feature = "prometheus")]
        if let Some(handle) = prom_handle {
            router = router.route(
                "/metrics",
                get(move || {
                    let h = handle.clone();
                    async move { h.render() }
                }),
            );
            info!("registered /metrics endpoint");
        }

        router
            .layer(middleware::from_fn(security_headers))
            .layer(DefaultBodyLimit::max(max_body_size))
    }

    /// Consumes the runtime and returns only the axum [`Router`].
    ///
    /// Cron jobs are **not** started. This is useful for testing the HTTP
    /// layer in isolation without side-effects (e.g. with
    /// `tower::ServiceExt::oneshot`).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::prelude::*;
    ///
    /// let router = Runtime::new()
    ///     .webhook("/hooks/test", WebhookAuth::none(), |_payload| async {})
    ///     .into_router();
    /// ```
    pub fn into_router(self) -> Router {
        if !self.crons.is_empty() {
            warn!(
                cron_count = self.crons.len(),
                "into_router() drops registered cron jobs - use serve() or run_crons() to start them"
            );
        }
        let tracker = Arc::new(HandlerTracker::new(self.max_concurrent_handlers));
        Self::build_router(
            self.webhooks,
            tracker,
            self.max_body_size,
            #[cfg(feature = "prometheus")]
            None,
        )
    }

    /// Starts the cron scheduler with all registered cron jobs.
    ///
    /// This is an internal helper used by both [`Runtime::serve`] and
    /// [`Runtime::run_crons`].
    async fn start_scheduler(crons: Vec<CronJob>) -> Result<JobScheduler, RuntimeError> {
        let scheduler = JobScheduler::new().await?;

        for cron_job in crons {
            let handler = Arc::new(cron_job.handler);
            let name = cron_job.name.clone();
            let running = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let job = Job::new_async(cron_job.schedule.as_str(), move |_uuid, _lock| {
                let handler = handler.clone();
                let name = name.clone();
                let running = running.clone();
                Box::pin(async move {
                    if running.swap(true, std::sync::atomic::Ordering::AcqRel) {
                        warn!(cron = %name, "cron job still running, skipping this tick");
                        return;
                    }
                    info!(cron = %name, "cron job triggered");
                    #[cfg(feature = "prometheus")]
                    metrics::counter!(metric_names::CRON_RUNS_TOTAL, "job" => name.clone())
                        .increment(1);
                    (handler)().await;
                    running.store(false, std::sync::atomic::Ordering::Release);
                })
            })?;
            info!(cron = %cron_job.name, schedule = %cron_job.schedule, "registered cron job");
            scheduler.add(job).await?;
        }

        scheduler.start().await?;
        Ok(scheduler)
    }

    /// Starts only the cron scheduler, blocking until a shutdown signal is
    /// received (`Ctrl+C` / `SIGTERM`).
    ///
    /// Unlike [`Runtime::serve`], this does **not** start an HTTP server. Any
    /// registered webhooks are ignored (a warning is logged if webhooks were
    /// registered).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// - The cron scheduler fails to initialise or a cron expression is invalid.
    /// - The scheduler fails to shut down cleanly.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), ironflow_runtime::error::RuntimeError> {
    ///     Runtime::new()
    ///         .cron("0 0 * * * *", "hourly-sync", || async {
    ///             println!("syncing...");
    ///         })
    ///         .run_crons()
    ///         .await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn run_crons(self) -> Result<(), RuntimeError> {
        let _ = dotenvy::dotenv();

        if !self.webhooks.is_empty() {
            warn!(
                webhook_count = self.webhooks.len(),
                "run_crons() ignores registered webhooks - use serve() to start both webhooks and crons"
            );
        }

        #[cfg(feature = "prometheus")]
        {
            match metrics_exporter_prometheus::PrometheusBuilder::new().install_recorder() {
                Ok(_) => info!("prometheus metrics recorder installed"),
                Err(_) => {
                    info!("prometheus metrics recorder already installed, reusing existing")
                }
            }
        }

        let mut scheduler = Self::start_scheduler(self.crons).await?;

        info!("ironflow cron scheduler running (no HTTP server)");
        match self.custom_shutdown {
            Some(signal) => signal.await,
            None => shutdown_signal().await,
        }

        info!("shutting down scheduler");
        scheduler.shutdown().await.map_err(RuntimeError::Shutdown)?;
        info!("ironflow cron scheduler stopped");

        Ok(())
    }

    /// Starts the HTTP server and cron scheduler, blocking until shutdown.
    ///
    /// This method:
    ///
    /// 1. Loads environment variables from `.env` via [`dotenvy`].
    /// 2. Starts the [`tokio_cron_scheduler`] scheduler with all registered cron jobs.
    /// 3. Builds an [Axum](https://docs.rs/axum) router with all registered webhook
    ///    routes plus a `GET /health` endpoint.
    /// 4. Binds to `addr` and serves until a `Ctrl+C` signal is received.
    /// 5. Gracefully shuts down the scheduler before returning.
    ///
    /// If you only need cron jobs without an HTTP server, use
    /// [`Runtime::run_crons`] instead.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// - The cron scheduler fails to initialise or a cron expression is invalid.
    /// - The TCP listener cannot bind to `addr`.
    /// - The Axum server encounters a fatal I/O error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), ironflow_runtime::error::RuntimeError> {
    ///     Runtime::new()
    ///         .serve("127.0.0.1:3000")
    ///         .await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn serve(self, addr: &str) -> Result<(), RuntimeError> {
        let _ = dotenvy::dotenv();

        #[cfg(feature = "prometheus")]
        let prom_handle = {
            match metrics_exporter_prometheus::PrometheusBuilder::new().install_recorder() {
                Ok(handle) => {
                    info!("prometheus metrics recorder installed");
                    Some(handle)
                }
                Err(_) => {
                    info!("prometheus metrics recorder already installed, reusing existing");
                    None
                }
            }
        };

        let mut scheduler = Self::start_scheduler(self.crons).await?;

        let tracker = Arc::new(HandlerTracker::new(self.max_concurrent_handlers));
        let router = Self::build_router(
            self.webhooks,
            tracker.clone(),
            self.max_body_size,
            #[cfg(feature = "prometheus")]
            prom_handle,
        );

        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(RuntimeError::Bind)?;
        info!(addr = %addr, "ironflow runtime listening");

        let graceful_shutdown = match self.custom_shutdown {
            Some(signal) => signal,
            None => Box::pin(shutdown_signal()),
        };
        axum::serve(listener, router)
            .with_graceful_shutdown(graceful_shutdown)
            .await
            .map_err(RuntimeError::Serve)?;

        // Wait for all in-flight webhook handlers to finish.
        info!("waiting for in-flight webhook handlers to complete");
        tracker.wait().await;

        info!("shutting down scheduler");
        scheduler.shutdown().await.map_err(RuntimeError::Shutdown)?;
        info!("ironflow runtime stopped");

        Ok(())
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks in-flight webhook handlers and enforces a concurrency limit.
///
/// Combines a [`Semaphore`] for backpressure with a [`JoinSet`] so that
/// [`Runtime::serve`] can wait for all running handlers before exiting.
struct HandlerTracker {
    semaphore: Arc<Semaphore>,
    join_set: Mutex<JoinSet<()>>,
}

impl HandlerTracker {
    fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            join_set: Mutex::new(JoinSet::new()),
        }
    }

    /// Spawn a handler task, respecting the concurrency limit.
    async fn spawn(&self, name: String, handler: WebhookHandler, ctx: WebhookContext) {
        let semaphore = self.semaphore.clone();
        let mut js = self.join_set.lock().await;
        // Reap completed tasks to detect panics early.
        while let Some(result) = js.try_join_next() {
            if let Err(e) = result {
                error!(error = %e, "webhook handler panicked");
            }
        }
        use tracing::Instrument;
        let span = tracing::info_span!("webhook", path = %name);
        js.spawn(
            async move {
                let _permit = semaphore
                    .acquire()
                    .await
                    .expect("semaphore closed unexpectedly");
                info!("webhook workflow started");
                handler(ctx).await;
                info!("webhook workflow completed");
            }
            .instrument(span),
        );
    }

    /// Wait for all in-flight handlers to complete.
    async fn wait(&self) {
        let mut js = self.join_set.lock().await;
        while let Some(result) = js.join_next().await {
            if let Err(e) = result {
                error!(error = %e, "webhook handler panicked");
            }
        }
    }
}

#[derive(Clone)]
struct WebhookState {
    auth: Arc<WebhookAuth>,
    handler: WebhookHandler,
    name: Arc<str>,
    tracker: Arc<HandlerTracker>,
}

async fn webhook_handler(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let name = &state.name;
    if !state.auth.verify(&headers, &body) {
        warn!(webhook = %name, "webhook auth failed");
        #[cfg(feature = "prometheus")]
        {
            let label: String = name.to_string();
            metrics::counter!(metric_names::WEBHOOK_RECEIVED_TOTAL, "path" => label, "auth" => metric_names::AUTH_REJECTED).increment(1);
        }
        return StatusCode::UNAUTHORIZED;
    }

    let payload: Value = match from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!(webhook = %name, error = %e, "invalid JSON body");
            #[cfg(feature = "prometheus")]
            {
                let label: String = name.to_string();
                metrics::counter!(metric_names::WEBHOOK_RECEIVED_TOTAL, "path" => label, "auth" => metric_names::AUTH_INVALID_BODY).increment(1);
            }
            return StatusCode::BAD_REQUEST;
        }
    };

    #[cfg(feature = "prometheus")]
    {
        let label: String = name.to_string();
        metrics::counter!(metric_names::WEBHOOK_RECEIVED_TOTAL, "path" => label, "auth" => metric_names::AUTH_ACCEPTED).increment(1);
    }

    let ctx = WebhookContext {
        payload,
        delivery_id: extract_delivery_id(&headers),
    };

    state
        .tracker
        .spawn(name.to_string(), state.handler.clone(), ctx)
        .await;

    StatusCode::ACCEPTED
}

async fn security_headers(
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff".parse().expect("valid header value"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        "DENY".parse().expect("valid header value"),
    );
    headers.insert(
        "x-xss-protection",
        "1; mode=block".parse().expect("valid header value"),
    );
    headers.insert(
        header::STRICT_TRANSPORT_SECURITY,
        "max-age=31536000; includeSubDomains"
            .parse()
            .expect("valid header value"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'".parse().expect("valid header value"),
    );
    response
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!("failed to install ctrl+c handler: {e}");
        }
    };

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            () = ctrl_c => info!("received SIGINT, shutting down"),
            _ = sigterm.recv() => info!("received SIGTERM, shutting down"),
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await;
        info!("received ctrl+c, shutting down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that Runtime::new() creates a runtime with default values.
    #[test]
    fn runtime_new_creates_with_defaults() {
        let rt = Runtime::new();
        assert_eq!(rt.webhooks.len(), 0);
        assert_eq!(rt.crons.len(), 0);
        assert_eq!(rt.max_body_size, DEFAULT_MAX_BODY_SIZE);
        assert_eq!(rt.max_concurrent_handlers, DEFAULT_MAX_CONCURRENT_HANDLERS);
        assert!(rt.custom_shutdown.is_none());
    }

    /// Test that Runtime::default() is equivalent to Runtime::new().
    #[test]
    fn runtime_default_equals_new() {
        let rt_new = Runtime::new();
        let rt_default = Runtime::default();
        assert_eq!(rt_new.webhooks.len(), rt_default.webhooks.len());
        assert_eq!(rt_new.crons.len(), rt_default.crons.len());
        assert_eq!(rt_new.max_body_size, rt_default.max_body_size);
        assert_eq!(
            rt_new.max_concurrent_handlers,
            rt_default.max_concurrent_handlers
        );
    }

    /// Test that max_body_size() builder method sets the value and returns self.
    #[test]
    fn max_body_size_sets_value_and_returns_self() {
        let rt = Runtime::new().max_body_size(512 * 1024);
        assert_eq!(rt.max_body_size, 512 * 1024);
    }

    /// Test that max_body_size() can be chained with other builder methods.
    #[test]
    fn max_body_size_chainable() {
        let rt =
            Runtime::new()
                .max_body_size(1024)
                .webhook("/test", WebhookAuth::none(), |_| async {});
        assert_eq!(rt.max_body_size, 1024);
        assert_eq!(rt.webhooks.len(), 1);
    }

    /// Test that max_body_size() can be set to zero.
    #[test]
    fn max_body_size_can_be_zero() {
        let rt = Runtime::new().max_body_size(0);
        assert_eq!(rt.max_body_size, 0);
    }

    /// Test that max_body_size() can be set to large values.
    #[test]
    fn max_body_size_can_be_large() {
        let large_size = 1024 * 1024 * 1024; // 1 GiB
        let rt = Runtime::new().max_body_size(large_size);
        assert_eq!(rt.max_body_size, large_size);
    }

    /// Test that max_concurrent_handlers() panics when given 0.
    #[test]
    #[should_panic(expected = "max_concurrent_handlers must be greater than 0")]
    fn max_concurrent_handlers_zero_panics() {
        let _ = Runtime::new().max_concurrent_handlers(0);
    }

    /// Test that max_concurrent_handlers() sets the value for valid inputs.
    #[test]
    fn max_concurrent_handlers_sets_valid_values() {
        let rt = Runtime::new().max_concurrent_handlers(16);
        assert_eq!(rt.max_concurrent_handlers, 16);
    }

    /// Test that max_concurrent_handlers() with 1 is allowed.
    #[test]
    fn max_concurrent_handlers_one_is_valid() {
        let rt = Runtime::new().max_concurrent_handlers(1);
        assert_eq!(rt.max_concurrent_handlers, 1);
    }

    /// Test that max_concurrent_handlers() with large values is allowed.
    #[test]
    fn max_concurrent_handlers_large_value_is_valid() {
        let large_limit = 10000;
        let rt = Runtime::new().max_concurrent_handlers(large_limit);
        assert_eq!(rt.max_concurrent_handlers, large_limit);
    }

    /// Test that max_concurrent_handlers() returns self for chaining.
    #[test]
    fn max_concurrent_handlers_chainable() {
        let rt = Runtime::new().max_concurrent_handlers(32).webhook(
            "/test",
            WebhookAuth::none(),
            |_| async {},
        );
        assert_eq!(rt.max_concurrent_handlers, 32);
        assert_eq!(rt.webhooks.len(), 1);
    }

    /// Test that with_shutdown() sets a custom shutdown signal and returns self.
    #[tokio::test]
    async fn with_shutdown_sets_signal_and_returns_self() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let rt = Runtime::new().with_shutdown(async move {
            let _ = rx.await;
        });
        assert!(rt.custom_shutdown.is_some());

        // Signal to verify it was set properly.
        let _ = tx.send(());
    }

    /// Test that with_shutdown() is chainable.
    #[tokio::test]
    async fn with_shutdown_chainable() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let rt = Runtime::new()
            .with_shutdown(async move {
                let _ = rx.await;
            })
            .webhook("/test", WebhookAuth::none(), |_| async {});
        assert!(rt.custom_shutdown.is_some());
        assert_eq!(rt.webhooks.len(), 1);

        let _ = tx.send(());
    }

    /// Test that webhook() registers a route and returns self.
    #[test]
    fn webhook_registers_route_and_returns_self() {
        let rt = Runtime::new().webhook("/hooks/test", WebhookAuth::none(), |_| async {});
        assert_eq!(rt.webhooks.len(), 1);
        assert_eq!(rt.webhooks[0].path, "/hooks/test");
    }

    /// Test that webhook() panics if path does not start with '/'.
    #[test]
    #[should_panic(expected = "webhook path must start with '/'")]
    fn webhook_path_without_slash_panics() {
        let _ = Runtime::new().webhook("no-slash", WebhookAuth::none(), |_| async {});
    }

    /// Test that webhook() accepts paths with various formats.
    #[test]
    fn webhook_accepts_valid_paths() {
        let rt = Runtime::new()
            .webhook("/", WebhookAuth::none(), |_| async {})
            .webhook("/simple", WebhookAuth::none(), |_| async {})
            .webhook("/nested/path", WebhookAuth::none(), |_| async {})
            .webhook("/with-dashes", WebhookAuth::none(), |_| async {})
            .webhook("/with_underscores", WebhookAuth::none(), |_| async {})
            .webhook("/with/numbers/123", WebhookAuth::none(), |_| async {});
        assert_eq!(rt.webhooks.len(), 6);
    }

    /// Test that webhook() can be chained multiple times.
    #[test]
    fn webhook_chainable() {
        let rt = Runtime::new()
            .webhook("/hook-a", WebhookAuth::none(), |_| async {})
            .webhook("/hook-b", WebhookAuth::none(), |_| async {})
            .webhook("/hook-c", WebhookAuth::none(), |_| async {});
        assert_eq!(rt.webhooks.len(), 3);
        assert_eq!(rt.webhooks[0].path, "/hook-a");
        assert_eq!(rt.webhooks[1].path, "/hook-b");
        assert_eq!(rt.webhooks[2].path, "/hook-c");
    }

    /// Test that webhook() works with different auth types.
    #[test]
    fn webhook_with_various_auth_types() {
        let rt = Runtime::new()
            .webhook("/none", WebhookAuth::none(), |_| async {})
            .webhook(
                "/header",
                WebhookAuth::header("x-api-key", "secret"),
                |_| async {},
            )
            .webhook("/github", WebhookAuth::github("secret"), |_| async {})
            .webhook("/gitlab", WebhookAuth::gitlab("token"), |_| async {});
        assert_eq!(rt.webhooks.len(), 4);
    }

    /// Test that cron() registers a job and returns self.
    #[test]
    fn cron_registers_job_and_returns_self() {
        let rt = Runtime::new().cron("0 0 * * * *", "daily-task", || async {});
        assert_eq!(rt.crons.len(), 1);
        assert_eq!(rt.crons[0].name, "daily-task");
        assert_eq!(rt.crons[0].schedule, "0 0 * * * *");
    }

    /// Test that cron() is chainable.
    #[test]
    fn cron_chainable() {
        let rt = Runtime::new()
            .cron("0 0 * * * *", "midnight", || async {})
            .cron("0 */5 * * * *", "every-5-minutes", || async {});
        assert_eq!(rt.crons.len(), 2);
    }

    /// Test that cron() preserves all parameters correctly.
    #[test]
    fn cron_preserves_schedule_and_name() {
        let rt = Runtime::new()
            .cron("0 12 * * * MON", "noon-mondays", || async {})
            .cron("0 0 1 * * *", "first-of-month", || async {});
        assert_eq!(rt.crons[0].name, "noon-mondays");
        assert_eq!(rt.crons[0].schedule, "0 12 * * * MON");
        assert_eq!(rt.crons[1].name, "first-of-month");
        assert_eq!(rt.crons[1].schedule, "0 0 1 * * *");
    }

    /// Test that into_router() returns a Router (compiles and doesn't panic).
    #[test]
    fn into_router_returns_router() {
        let rt = Runtime::new();
        let _router = rt.into_router();
        // If this compiles and doesn't panic, the router was successfully created.
    }

    /// Test that into_router() with webhooks returns a Router.
    #[test]
    fn into_router_with_webhooks_returns_router() {
        let rt = Runtime::new()
            .webhook("/hook-a", WebhookAuth::none(), |_| async {})
            .webhook("/hook-b", WebhookAuth::github("secret"), |_| async {});
        let _router = rt.into_router();
        // If this compiles and doesn't panic, the router was successfully created with all webhooks.
    }

    /// Test that into_router() with cron jobs (warns but doesn't panic).
    #[test]
    fn into_router_with_crons_returns_router() {
        let rt = Runtime::new()
            .cron("0 0 * * * *", "daily", || async {})
            .cron("0 */5 * * * *", "every-5-min", || async {});
        let _router = rt.into_router();
        // Crons are dropped but not an error; router should still be created.
    }

    /// Test that into_router() with max_body_size returns a Router.
    #[test]
    fn into_router_respects_max_body_size_config() {
        let rt =
            Runtime::new()
                .max_body_size(100)
                .webhook("/hook", WebhookAuth::none(), |_| async {});
        let _router = rt.into_router();
        // Router created; actual body size limit enforcement is tested in integration tests.
    }

    /// Test that into_router() with max_concurrent_handlers returns a Router.
    #[test]
    fn into_router_respects_max_concurrent_handlers_config() {
        let rt = Runtime::new().max_concurrent_handlers(16).webhook(
            "/hook",
            WebhookAuth::none(),
            |_| async {},
        );
        let _router = rt.into_router();
        // Router created; concurrency limit enforcement is tested in integration tests.
    }

    /// Test full builder chain with multiple methods.
    #[test]
    fn builder_chain_multiple_methods() {
        let rt = Runtime::new()
            .max_body_size(512 * 1024)
            .max_concurrent_handlers(32)
            .webhook("/hook-a", WebhookAuth::none(), |_| async {})
            .webhook("/hook-b", WebhookAuth::github("secret"), |_| async {})
            .cron("0 0 * * * *", "daily", || async {});

        assert_eq!(rt.max_body_size, 512 * 1024);
        assert_eq!(rt.max_concurrent_handlers, 32);
        assert_eq!(rt.webhooks.len(), 2);
        assert_eq!(rt.crons.len(), 1);
    }

    /// Test that into_router() drops cron jobs with a warning logged.
    #[test]
    fn into_router_with_crons_doesnt_start_them() {
        let rt = Runtime::new().cron("0 0 * * * *", "test-cron", || async {});
        // This should not panic; crons are simply dropped.
        let _router = rt.into_router();
    }
}
