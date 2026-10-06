//! `POST /api/v1/runs` — Trigger a workflow.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::engine::EnqueueOptions;
use ironflow_engine::error::EngineError;
use ironflow_engine::notify::{Event, RunCreatedEvent};
use ironflow_store::models::{ConcurrencyLimit, Run, RunCreation, TriggerKind};
use serde_json::{Value, json};
use tracing::{info, warn};

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::RUN_IDEMPOTENCY_TOTAL;
#[cfg(feature = "prometheus")]
use metrics::counter;

use ironflow_core::metric_names::{
    IDEMPOTENCY_CONFLICT, IDEMPOTENCY_CREATED, IDEMPOTENCY_REPLAYED,
};

use crate::actor::run_actor_of;
use crate::entities::{CreateRunRequest, RunResponse, validate_idempotency_key};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Header carrying the client-supplied idempotency key.
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Whether a replayed key was used for the same request as the run it is bound to.
///
/// The workflow, the payload, the concurrency key and the concurrency limits are
/// compared: labels are merged with the handler's defaults at enqueue time, so
/// comparing them would turn a handler version bump into a spurious conflict.
/// Worker tags are left out for the same reason: the run carries the handler's
/// required tags merged with the request's.
fn same_request(
    existing: &Run,
    workflow: &str,
    payload: &Value,
    concurrency_key: Option<&str>,
    concurrency_limits: &[ConcurrencyLimit],
) -> bool {
    existing.workflow_name == workflow
        && &existing.payload == payload
        && existing.concurrency_key.as_deref() == concurrency_key
        && existing.concurrency_limits == concurrency_limits
}

#[cfg(feature = "prometheus")]
fn record_outcome(outcome: &'static str) {
    counter!(RUN_IDEMPOTENCY_TOTAL, "outcome" => outcome).increment(1);
}

#[cfg(not(feature = "prometheus"))]
fn record_outcome(_outcome: &'static str) {}

/// Trigger a workflow by name.
///
/// Returns 201 Created with the newly enqueued run.
///
/// An optional `Idempotency-Key` header makes the call safe to replay: the same
/// key returns the run it already produced with 200 OK instead of enqueueing a
/// second one. A key reused with a different workflow or payload is rejected with
/// 409 Conflict. Keys stay bound for 24 hours, after which they are released.
///
/// An optional `concurrency_key` in the body makes the run exclusive: while a
/// non-terminal run holds the same key, the call is refused with 409
/// `CONCURRENCY_CONFLICT` naming that run.
///
/// Optional `concurrency_limits` in the body put the run in concurrency groups:
/// it is created at once but a worker only starts it while, for each group,
/// fewer root runs of that group than its limit are running.
///
/// Optional `worker_tags` in the body are added to the tags the workflow
/// requires: only a worker carrying all of them takes the run.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] for non-admin callers.
/// Returns [`ApiError::BadRequest`] if the workflow is unknown, the body is
/// invalid (including malformed `concurrency_limits` or `worker_tags`), or the
/// `Idempotency-Key` header is malformed.
/// Returns [`ApiError::IdempotencyKeyConflict`] if the key is bound to a
/// different request.
/// Returns [`ApiError::ConcurrencyConflict`] if a non-terminal run already
/// holds the requested `concurrency_key`.
/// Returns [`ApiError::MonthlyBudgetExceeded`] if the global monthly cost quota
/// is exhausted.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs",
        tags = ["runs"],
        request_body(content = CreateRunRequest, description = "Workflow to trigger"),
        params(
            ("Idempotency-Key" = Option<String>, Header, description = "Optional key making the call safe to replay. At most 255 printable ASCII characters, valid for 24 hours.")
        ),
        responses(
            (status = 201, description = "Run created successfully", body = RunResponse),
            (status = 200, description = "Idempotency key replayed: the existing run is returned", body = RunResponse),
            (status = 400, description = "Unknown workflow, invalid body (including malformed concurrency_limits or worker_tags) or malformed Idempotency-Key"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 409, description = "Idempotency key already used with a different request (IDEMPOTENCY_KEY_CONFLICT), or concurrency key held by an active run (CONCURRENCY_CONFLICT)"),
            (status = 429, description = "Monthly cost quota exhausted")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn create_run(
    auth: Authenticated,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let idempotency_key = match headers.get(IDEMPOTENCY_KEY_HEADER) {
        Some(value) => {
            let key = value.to_str().map_err(|_| {
                ApiError::BadRequest(
                    "Idempotency-Key must contain only printable ASCII characters".to_string(),
                )
            })?;
            validate_idempotency_key(key).map_err(|e| ApiError::BadRequest(e.message()))?;
            Some(key.to_string())
        }
        None => None,
    };

    // Validated before any write, so an unknown workflow never consumes the key.
    if !state
        .engine
        .handler_names()
        .contains(&req.workflow.as_str())
    {
        return Err(ApiError::BadRequest(format!(
            "unknown workflow: {}",
            req.workflow
        )));
    }

    req.validate().map_err(ApiError::BadRequest)?;

    let payload = req.payload.unwrap_or_else(|| json!({}));
    let labels = req.labels.unwrap_or_default();

    let creation = state
        .engine
        .enqueue_handler_with_options(
            &req.workflow,
            TriggerKind::Api,
            payload.clone(),
            EnqueueOptions {
                max_retries: req.max_retries.unwrap_or(0),
                labels,
                scheduled_at: req.scheduled_at,
                max_cost_usd: req.max_cost_usd,
                created_by: Some(run_actor_of(&auth)),
                idempotency_key: idempotency_key.clone(),
                concurrency_key: req.concurrency_key.clone(),
                concurrency_limits: req.concurrency_limits.clone(),
                worker_tags: req.worker_tags,
            },
        )
        .await
        .map_err(|e| match e {
            EngineError::MonthlyBudgetExceeded { .. } => {
                ApiError::MonthlyBudgetExceeded(e.to_string())
            }
            EngineError::ConcurrencyConflict { key, run_id } => {
                ApiError::ConcurrencyConflict { key, run_id }
            }
            EngineError::InvalidConcurrencyLimit(e) => ApiError::BadRequest(e.to_string()),
            EngineError::InvalidWorkerTag(e) => ApiError::BadRequest(e.to_string()),
            other => ApiError::Internal(other.to_string()),
        })?;

    match creation {
        RunCreation::Existing(existing) => {
            if !same_request(
                &existing,
                &req.workflow,
                &payload,
                req.concurrency_key.as_deref(),
                &req.concurrency_limits,
            ) {
                warn!(
                    idempotency_key = idempotency_key.as_deref().unwrap_or(""),
                    run_id = %existing.id,
                    "idempotency key reused with a different request"
                );
                record_outcome(IDEMPOTENCY_CONFLICT);
                return Err(ApiError::IdempotencyKeyConflict(existing.id));
            }

            info!(
                idempotency_key = idempotency_key.as_deref().unwrap_or(""),
                run_id = %existing.id,
                "idempotent replay, returning the original run"
            );
            record_outcome(IDEMPOTENCY_REPLAYED);

            // No RunCreated event: nothing was enqueued.
            Ok((StatusCode::OK, ok(RunResponse::from(existing))))
        }
        RunCreation::Created(run) => {
            if idempotency_key.is_some() {
                record_outcome(IDEMPOTENCY_CREATED);
            }

            state
                .engine
                .event_publisher()
                .publish(Event::RunCreated(RunCreatedEvent {
                    run_id: run.id,
                    workflow_name: run.workflow_name.clone(),
                    at: Utc::now(),
                }));

            Ok((StatusCode::CREATED, ok(RunResponse::from(run))))
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
    use ironflow_auth::jwt::AccessToken;
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::budget::BudgetConfig;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::{Event, EventSubscriber, SubscriberFuture};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{ApiKeyScope, NewApiKey, NewUser, RunFilter, RunStatus};
    use rust_decimal::Decimal;
    use serde_json::{Value as JsonValue, json};
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::routes::test_helpers::create_user_auth_header;

    struct TestWorkflow;

    impl WorkflowHandler for TestWorkflow {
        fn name(&self) -> &str {
            "test-workflow"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct OtherWorkflow;

    impl WorkflowHandler for OtherWorkflow {
        fn name(&self) -> &str {
            "other-workflow"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct GpuWorkflow;

    impl WorkflowHandler for GpuWorkflow {
        fn name(&self) -> &str {
            "gpu-workflow"
        }

        fn required_worker_tags(&self) -> Vec<String> {
            vec!["gpu".to_string()]
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    /// Counts the `RunCreated` events the engine actually broadcasts.
    struct RunCreatedCounter(Arc<AtomicUsize>);

    impl EventSubscriber for RunCreatedCounter {
        fn name(&self) -> &str {
            "run-created-counter"
        }

        fn handle<'a>(&'a self, _event: &'a Event) -> SubscriberFuture<'a> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {})
        }
    }

    fn build_state(counter: Option<Arc<AtomicUsize>>, budget: BudgetConfig) -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider).with_budget_config(budget);
        engine.register(TestWorkflow).unwrap();
        engine.register(OtherWorkflow).unwrap();
        engine.register(GpuWorkflow).unwrap();
        if let Some(counter) = counter {
            engine.subscribe(RunCreatedCounter(counter), &[Event::RUN_CREATED]);
        }
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(16);
        AppState::new(
            store,
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn test_state_counting_created() -> (AppState, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        (
            build_state(Some(counter.clone()), BudgetConfig::new()),
            counter,
        )
    }

    fn test_state() -> AppState {
        build_state(None, BudgetConfig::new())
    }

    fn state_with_budget(budget: BudgetConfig) -> AppState {
        build_state(None, budget)
    }

    /// Build a `POST /` request, optionally carrying an `Idempotency-Key`.
    fn post_run(auth_header: &str, body: JsonValue, key: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header);
        if let Some(key) = key {
            builder = builder.header("idempotency-key", key);
        }
        builder
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap()
    }

    async fn body_json(resp: axum::response::Response) -> JsonValue {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn router(state: AppState) -> Router {
        Router::new().route("/", post(create_run)).with_state(state)
    }

    #[tokio::test]
    async fn create_run_success() {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", true).await;
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({
                    "workflow": "test-workflow",
                    "payload": {"key": "value"}
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["workflow_name"], "test-workflow");
    }

    #[tokio::test]
    async fn create_run_defaults_to_no_automatic_retry() {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", true).await;
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({"workflow": "test-workflow"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["max_retries"], 0);
    }

    #[tokio::test]
    async fn create_run_honours_max_retries() {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", true).await;
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({
                    "workflow": "test-workflow",
                    "max_retries": 2
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["max_retries"], 2);
    }

    #[tokio::test]
    async fn create_run_unknown_workflow() {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", true).await;
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({
                    "workflow": "unknown-workflow",
                    "payload": {}
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// Send a `POST /` with the given JSON body against a router built on `state`.
    async fn send_run(state: AppState, body: JsonValue) -> Response<Body> {
        let auth_header = create_user_auth_header(&state, "testuser", true).await;
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        app.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn create_run_persists_and_returns_max_cost_usd() {
        let resp = send_run(
            test_state(),
            json!({"workflow": "test-workflow", "max_cost_usd": 2.5}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["max_cost_usd"], 2.5);
    }

    #[tokio::test]
    async fn create_run_without_max_cost_omits_the_field() {
        let resp = send_run(test_state(), json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert!(json_val["data"].get("max_cost_usd").is_none());
    }

    #[tokio::test]
    async fn create_run_applies_server_default_max_cost() {
        let state =
            state_with_budget(BudgetConfig::new().default_run_max_cost_usd(Decimal::new(125, 2)));
        let resp = send_run(state, json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["max_cost_usd"], 1.25);
    }

    #[tokio::test]
    async fn create_run_rejects_negative_max_cost() {
        let resp = send_run(
            test_state(),
            json!({"workflow": "test-workflow", "max_cost_usd": -1.0}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "BAD_REQUEST");
        assert!(
            json_val["error"]["message"]
                .as_str()
                .unwrap()
                .contains("max_cost_usd")
        );
    }

    #[tokio::test]
    async fn create_run_accepts_zero_max_cost() {
        let resp = send_run(
            test_state(),
            json!({"workflow": "test-workflow", "max_cost_usd": 0}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_run_returns_429_when_monthly_quota_exhausted() {
        // Quota of $0: any accumulated cost (including zero) meets the limit.
        let state = state_with_budget(BudgetConfig::new().monthly_cost_limit_usd(Decimal::ZERO));
        let resp = send_run(state, json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "MONTHLY_BUDGET_EXCEEDED");
    }

    #[tokio::test]
    async fn create_run_succeeds_when_monthly_quota_has_room() {
        let state =
            state_with_budget(BudgetConfig::new().monthly_cost_limit_usd(Decimal::new(10000, 2)));
        let resp = send_run(state, json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_run_without_payload() {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", true).await;
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({
                    "workflow": "test-workflow"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // ---- created_by ----

    /// Seed an admin user and return `(user_id, username)`.
    async fn seed_admin(state: &AppState, username: &str) -> (Uuid, String) {
        let user = state
            .store
            .create_user(NewUser {
                email: format!("{username}@example.com"),
                username: username.to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(true),
            })
            .await
            .expect("create user");
        (user.id, user.username)
    }

    async fn post_create_run(state: AppState, auth_header: &str) -> JsonValue {
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({"workflow": "test-workflow"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn create_run_records_the_jwt_user_as_author() {
        let state = test_state();
        let (user_id, username) = seed_admin(&state, "alice").await;
        let token = AccessToken::for_user(user_id, &username, true, &state.jwt_config).unwrap();

        let body = post_create_run(state, &format!("Bearer {}", token.0)).await;

        assert_eq!(body["data"]["created_by"]["kind"], "user");
        assert_eq!(body["data"]["created_by"]["id"], user_id.to_string());
        assert_eq!(body["data"]["created_by"]["label"], "alice");
    }

    #[tokio::test]
    async fn create_run_records_the_api_key_as_author() {
        let state = test_state();
        let (user_id, _) = seed_admin(&state, "alice").await;

        let raw_key = format!("{API_KEY_PREFIX}0123456789abcdef");
        let key = state
            .store
            .create_api_key(NewApiKey {
                user_id,
                name: "ci-deploy".to_string(),
                key_hash: password::hash(&raw_key).unwrap(),
                key_prefix: raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN].to_string(),
                scopes: vec![ApiKeyScope::RunsWrite],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .expect("create api key");

        let body = post_create_run(state, &format!("Bearer {raw_key}")).await;

        assert_eq!(body["data"]["created_by"]["kind"], "api_key");
        assert_eq!(body["data"]["created_by"]["id"], key.id.to_string());
        assert_eq!(body["data"]["created_by"]["label"], "ci-deploy (alice)");
    }

    #[tokio::test]
    async fn create_run_author_label_is_the_username_of_the_stored_user() {
        // Tokens of unknown users are rejected, so the author is always a stored user.
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", true).await;

        let body = post_create_run(state, &auth_header).await;

        assert_eq!(body["data"]["created_by"]["kind"], "user");
        assert_eq!(body["data"]["created_by"]["label"], "testuser");
    }

    // ---- Idempotency-Key ----

    fn deploy_body() -> JsonValue {
        json!({"workflow": "test-workflow", "payload": {"env": "prod"}})
    }

    #[tokio::test]
    async fn without_header_always_creates_a_new_run() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let first = router(state.clone())
            .oneshot(post_run(&auth, deploy_body(), None))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::CREATED);
        let first_id = body_json(first).await["data"]["id"].clone();

        let second = router(state)
            .oneshot(post_run(&auth, deploy_body(), None))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::CREATED);
        let second_id = body_json(second).await["data"]["id"].clone();

        assert_ne!(first_id, second_id);
    }

    #[tokio::test]
    async fn without_header_response_omits_the_key() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let resp = router(state)
            .oneshot(post_run(&auth, deploy_body(), None))
            .await
            .unwrap();

        let body = body_json(resp).await;
        assert!(body["data"].get("idempotency_key").is_none());
    }

    #[tokio::test]
    async fn first_call_with_a_key_creates_the_run() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let resp = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some("github:abc-123")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = body_json(resp).await;
        assert_eq!(body["data"]["idempotency_key"], "github:abc-123");
    }

    #[tokio::test]
    async fn replayed_key_returns_200_and_the_original_run() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let first = router(state.clone())
            .oneshot(post_run(&auth, deploy_body(), Some("github:abc-123")))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::CREATED);
        let first_id = body_json(first).await["data"]["id"].clone();

        let second = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some("github:abc-123")))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::OK);
        let second_id = body_json(second).await["data"]["id"].clone();

        assert_eq!(first_id, second_id);
    }

    #[tokio::test]
    async fn replayed_key_with_a_different_payload_conflicts() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let first = router(state.clone())
            .oneshot(post_run(&auth, deploy_body(), Some("github:abc-123")))
            .await
            .unwrap();
        let first_id = body_json(first).await["data"]["id"].clone();

        let other_payload = json!({"workflow": "test-workflow", "payload": {"env": "staging"}});
        let second = router(state)
            .oneshot(post_run(&auth, other_payload, Some("github:abc-123")))
            .await
            .unwrap();

        assert_eq!(second.status(), StatusCode::CONFLICT);
        let body = body_json(second).await;
        assert_eq!(body["error"]["code"], "IDEMPOTENCY_KEY_CONFLICT");
        assert_eq!(body["error"]["details"]["run_id"], first_id);
    }

    #[tokio::test]
    async fn replayed_key_with_a_different_workflow_conflicts() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        router(state.clone())
            .oneshot(post_run(&auth, deploy_body(), Some("shared-key")))
            .await
            .unwrap();

        let other_workflow = json!({"workflow": "other-workflow", "payload": {"env": "prod"}});
        let second = router(state)
            .oneshot(post_run(&auth, other_workflow, Some("shared-key")))
            .await
            .unwrap();

        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn replay_returns_the_run_even_in_a_terminal_state() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let first = router(state.clone())
            .oneshot(post_run(&auth, deploy_body(), Some("github:abc-123")))
            .await
            .unwrap();
        let body = body_json(first).await;
        let run_id: Uuid = serde_json::from_value(body["data"]["id"].clone()).unwrap();

        state
            .store
            .update_run_status(run_id, RunStatus::Running)
            .await
            .unwrap();
        state
            .store
            .update_run_status(run_id, RunStatus::Failed)
            .await
            .unwrap();

        let replay = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some("github:abc-123")))
            .await
            .unwrap();

        assert_eq!(replay.status(), StatusCode::OK);
        let body = body_json(replay).await;
        assert_eq!(body["data"]["status"], "failed");
    }

    #[tokio::test]
    async fn empty_key_is_rejected() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let resp = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some("")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn key_at_the_length_limit_is_accepted() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;
        let key = "a".repeat(255);

        let resp = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some(&key)))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn key_over_the_length_limit_is_rejected() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;
        let key = "a".repeat(256);

        let resp = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some(&key)))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn non_ascii_key_is_rejected() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let resp = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some("cle-\u{e9}")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_workflow_does_not_consume_the_key() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let unknown = json!({"workflow": "nope", "payload": {"env": "prod"}});
        let rejected = router(state.clone())
            .oneshot(post_run(&auth, unknown, Some("github:abc-123")))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

        // The same key is still free for a valid request.
        let accepted = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some("github:abc-123")))
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn replay_publishes_a_single_run_created_event() {
        let (state, created_events) = test_state_counting_created();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        for _ in 0..3 {
            router(state.clone())
                .oneshot(post_run(&auth, deploy_body(), Some("github:abc-123")))
                .await
                .unwrap();
        }

        // Subscribers run in spawned tasks; yield until they have all run.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(
            created_events.load(Ordering::SeqCst),
            1,
            "only the real creation should publish an event"
        );
    }

    #[tokio::test]
    async fn every_distinct_key_publishes_its_own_event() {
        let (state, created_events) = test_state_counting_created();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        for i in 0..3 {
            router(state.clone())
                .oneshot(post_run(&auth, deploy_body(), Some(&format!("key-{i}"))))
                .await
                .unwrap();
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(created_events.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn concurrent_calls_with_the_same_key_create_one_run() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let mut handles = Vec::new();
        for _ in 0..20 {
            let state = state.clone();
            let auth = auth.clone();
            handles.push(tokio::spawn(async move {
                router(state)
                    .oneshot(post_run(&auth, deploy_body(), Some("github:race")))
                    .await
                    .unwrap()
            }));
        }

        let mut created = 0;
        let mut ids = std::collections::HashSet::new();
        for handle in handles {
            let resp = handle.await.unwrap();
            let status = resp.status();
            assert!(
                status == StatusCode::CREATED || status == StatusCode::OK,
                "unexpected status {status}"
            );
            if status == StatusCode::CREATED {
                created += 1;
            }
            ids.insert(body_json(resp).await["data"]["id"].to_string());
        }

        assert_eq!(created, 1);
        assert_eq!(ids.len(), 1);
    }
    fn keyed_body(workflow: &str, key: &str) -> JsonValue {
        json!({"workflow": workflow, "concurrency_key": key})
    }

    #[tokio::test]
    async fn create_run_exposes_concurrency_key() {
        let resp = send_run(test_state(), keyed_body("test-workflow", "issue:12")).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = body_json(resp).await;
        assert_eq!(body["data"]["concurrency_key"], "issue:12");
    }

    #[tokio::test]
    async fn create_run_without_concurrency_key_omits_the_field() {
        let resp = send_run(test_state(), json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = body_json(resp).await;
        assert!(body["data"].get("concurrency_key").is_none());
    }

    #[tokio::test]
    async fn create_run_returns_409_on_concurrency_conflict() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let first = router(state.clone())
            .oneshot(post_run(
                &auth,
                keyed_body("test-workflow", "issue:12"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::CREATED);
        let first_id = body_json(first).await["data"]["id"].clone();

        // The key is global: another workflow cannot take it either.
        for workflow in ["test-workflow", "other-workflow"] {
            let resp = router(state.clone())
                .oneshot(post_run(&auth, keyed_body(workflow, "issue:12"), None))
                .await
                .unwrap();

            assert_eq!(resp.status(), StatusCode::CONFLICT);
            let body = body_json(resp).await;
            assert_eq!(body["error"]["code"], "CONCURRENCY_CONFLICT");
            assert_eq!(body["error"]["details"]["key"], "issue:12");
            assert_eq!(body["error"]["details"]["run_id"], first_id);
        }

        let other_key = router(state.clone())
            .oneshot(post_run(
                &auth,
                keyed_body("test-workflow", "issue:13"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(other_key.status(), StatusCode::CREATED);

        let runs = state
            .store
            .list_runs(RunFilter::default(), 1, 50)
            .await
            .unwrap();
        assert_eq!(runs.items.len(), 2, "a conflict creates no run");
    }

    #[tokio::test]
    async fn create_run_accepts_a_concurrency_key_once_its_holder_is_terminal() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        let first = router(state.clone())
            .oneshot(post_run(
                &auth,
                keyed_body("test-workflow", "issue:12"),
                None,
            ))
            .await
            .unwrap();
        let first_body = body_json(first).await;
        let first_id = Uuid::parse_str(first_body["data"]["id"].as_str().unwrap()).unwrap();
        state
            .store
            .update_run_status(first_id, RunStatus::Cancelled)
            .await
            .unwrap();

        let second = router(state)
            .oneshot(post_run(
                &auth,
                keyed_body("test-workflow", "issue:12"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_run_rejects_empty_concurrency_key() {
        for key in ["", "   "] {
            let resp = send_run(test_state(), keyed_body("test-workflow", key)).await;
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

            let body = body_json(resp).await;
            assert_eq!(body["error"]["code"], "BAD_REQUEST");
            assert_eq!(
                body["error"]["message"],
                "concurrency_key must not be empty"
            );
        }
    }

    #[tokio::test]
    async fn create_run_rejects_concurrency_key_over_the_limit() {
        let key = "k".repeat(256);
        let resp = send_run(test_state(), keyed_body("test-workflow", &key)).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = body_json(resp).await;
        assert_eq!(
            body["error"]["message"],
            "concurrency_key must be at most 255 bytes"
        );
    }

    #[tokio::test]
    async fn idempotent_replay_with_a_concurrency_key_returns_the_run() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;
        let body = keyed_body("test-workflow", "issue:12");

        let first = router(state.clone())
            .oneshot(post_run(&auth, body.clone(), Some("github:abc-123")))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::CREATED);
        let first_id = body_json(first).await["data"]["id"].clone();

        // The replay is answered by the idempotency key before the
        // concurrency key is checked: the run holding it is the same one.
        let replay = router(state)
            .oneshot(post_run(&auth, body, Some("github:abc-123")))
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(body_json(replay).await["data"]["id"], first_id);
    }

    #[tokio::test]
    async fn replayed_key_adding_a_concurrency_key_conflicts() -> Result<(), Infallible> {
        // The original run holds no key: replaying it would report an exclusive
        // run on "issue:12" that does not exist.
        let unkeyed = json!({"workflow": "test-workflow"});
        assert_replay_conflicts(unkeyed, keyed_body("test-workflow", "issue:12")).await
    }

    #[tokio::test]
    async fn replayed_key_changing_the_concurrency_key_conflicts() -> Result<(), Infallible> {
        // The original run holds "issue:12": replaying it would report an
        // exclusive run on "issue:13" while that key is still free.
        let original = keyed_body("test-workflow", "issue:12");
        assert_replay_conflicts(original, keyed_body("test-workflow", "issue:13")).await
    }

    fn limited_body(limit: u32) -> JsonValue {
        json!({
            "workflow": "test-workflow",
            "concurrency_limits": [{"group": "repo:acme", "limit": limit}],
        })
    }

    #[tokio::test]
    async fn create_run_exposes_concurrency_limits() {
        let resp = send_run(test_state(), limited_body(2)).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = body_json(resp).await;
        assert_eq!(
            body["data"]["concurrency_limits"],
            json!([{"group": "repo:acme", "limit": 2}])
        );
    }

    #[tokio::test]
    async fn create_run_without_concurrency_limits_omits_the_field() {
        let resp = send_run(test_state(), json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = body_json(resp).await;
        assert!(body["data"].get("concurrency_limits").is_none());
    }

    #[tokio::test]
    async fn create_run_in_a_saturated_group_is_still_created() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;

        // Unlike a concurrency key, a group never refuses a run: the second
        // one waits in the queue until the first one frees the slot.
        for _ in 0..2 {
            let resp = router(state.clone())
                .oneshot(post_run(&auth, limited_body(1), None))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::CREATED);
            assert_eq!(body_json(resp).await["data"]["status"], "pending");
        }
    }

    /// Sends `body`, asserts it is refused with 400 BAD_REQUEST and `expected`
    /// as message, and that no run was created.
    async fn assert_rejected_concurrency_limits(body: JsonValue, expected: &str) {
        let state = test_state();
        let resp = send_run(state.clone(), body).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
        assert_eq!(body["error"]["message"], expected);

        let runs = state
            .store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .unwrap();
        assert_eq!(runs.total, 0, "an invalid request creates no run");
    }

    #[tokio::test]
    async fn create_run_rejects_zero_concurrency_limit() {
        assert_rejected_concurrency_limits(
            limited_body(0),
            "concurrency_limits: concurrency limit for group 'repo:acme' must be at least 1",
        )
        .await;
    }

    #[tokio::test]
    async fn create_run_rejects_empty_concurrency_group() {
        assert_rejected_concurrency_limits(
            json!({
                "workflow": "test-workflow",
                "concurrency_limits": [{"group": "", "limit": 1}],
            }),
            "concurrency_limits: concurrency group must not be empty",
        )
        .await;
    }

    #[tokio::test]
    async fn create_run_rejects_duplicate_concurrency_group() {
        assert_rejected_concurrency_limits(
            json!({
                "workflow": "test-workflow",
                "concurrency_limits": [
                    {"group": "repo:acme", "limit": 1},
                    {"group": "repo:acme", "limit": 2},
                ],
            }),
            "concurrency_limits: concurrency group 'repo:acme' is listed more than once",
        )
        .await;
    }

    #[tokio::test]
    async fn replayed_key_changing_the_concurrency_limits_conflicts() -> Result<(), Infallible> {
        // The original run allows two runs of the group: replaying it would
        // report a run limited to one that does not exist.
        assert_replay_conflicts(limited_body(2), limited_body(1)).await
    }

    #[tokio::test]
    async fn create_run_without_worker_tags_returns_an_empty_list() {
        let resp = send_run(test_state(), json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(body_json(resp).await["data"]["worker_tags"], json!([]));
    }

    #[tokio::test]
    async fn create_run_merges_request_worker_tags_with_the_workflow_ones() {
        let state = test_state();
        let resp = send_run(
            state.clone(),
            json!({"workflow": "gpu-workflow", "worker_tags": ["region:eu", "gpu"]}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = body_json(resp).await;
        assert_eq!(body["data"]["worker_tags"], json!(["gpu", "region:eu"]));

        let run_id: Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();
        let stored = state.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(
            stored.worker_tags,
            vec!["gpu".to_string(), "region:eu".to_string()]
        );
    }

    #[tokio::test]
    async fn create_run_records_the_workflow_worker_tags() {
        let resp = send_run(test_state(), json!({"workflow": "gpu-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(body_json(resp).await["data"]["worker_tags"], json!(["gpu"]));
    }

    #[tokio::test]
    async fn create_run_rejects_invalid_worker_tag() {
        let state = test_state();
        let resp = send_run(
            state.clone(),
            json!({"workflow": "test-workflow", "worker_tags": ["two words"]}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains("worker_tags"), "{message}");

        let page = state
            .store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .unwrap();
        assert_eq!(page.total, 0, "no run may be created");
    }

    /// Creates a run from `original`, replays its idempotency key with `replay`
    /// and asserts the replay is refused with a conflict naming the original run.
    async fn assert_replay_conflicts(
        original: JsonValue,
        replay: JsonValue,
    ) -> Result<(), Infallible> {
        let state = test_state();
        let auth = create_user_auth_header(&state, "testuser", true).await;
        let first = router(state.clone())
            .oneshot(post_run(&auth, original, Some("github:abc-123")))
            .await?;
        assert_eq!(first.status(), StatusCode::CREATED);
        let first_id = body_json(first).await["data"]["id"].clone();

        let second = router(state)
            .oneshot(post_run(&auth, replay, Some("github:abc-123")))
            .await?;
        assert_eq!(second.status(), StatusCode::CONFLICT);
        let body = body_json(second).await;
        assert_eq!(body["error"]["code"], "IDEMPOTENCY_KEY_CONFLICT");
        assert_eq!(body["error"]["details"]["run_id"], first_id);
        Ok(())
    }
}
