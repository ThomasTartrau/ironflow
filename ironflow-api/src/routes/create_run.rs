//! `POST /api/v1/runs` — Trigger a workflow.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::Event;
use ironflow_store::models::{Run, RunCreation, TriggerKind};
use serde_json::{Value, json};
use tracing::{info, warn};

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::RUN_IDEMPOTENCY_TOTAL;
#[cfg(feature = "prometheus")]
use metrics::counter;

use ironflow_core::metric_names::{
    IDEMPOTENCY_CONFLICT, IDEMPOTENCY_CREATED, IDEMPOTENCY_REPLAYED,
};

use crate::entities::{CreateRunRequest, RunResponse, validate_idempotency_key};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Header carrying the client-supplied idempotency key.
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Whether a replayed key was used for the same request as the run it is bound to.
///
/// Only the workflow and the payload are compared: labels are merged with the
/// handler's defaults at enqueue time, so comparing them would turn a handler
/// version bump into a spurious conflict.
fn same_request(existing: &Run, workflow: &str, payload: &Value) -> bool {
    existing.workflow_name == workflow && &existing.payload == payload
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
/// # Errors
///
/// Returns [`ApiError::Forbidden`] for non-admin callers.
/// Returns [`ApiError::BadRequest`] if the workflow is unknown or the
/// `Idempotency-Key` header is malformed.
/// Returns [`ApiError::IdempotencyKeyConflict`] if the key is bound to a
/// different request.
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
            (status = 400, description = "Unknown workflow or malformed Idempotency-Key"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 409, description = "Idempotency key already used with a different request")
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

    let payload = req.payload.unwrap_or_else(|| json!({}));
    let labels = req.labels.unwrap_or_default();

    let creation = state
        .engine
        .enqueue_handler_with_options(
            &req.workflow,
            TriggerKind::Api,
            payload.clone(),
            3,
            labels,
            req.scheduled_at,
            idempotency_key.clone(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    match creation {
        RunCreation::Existing(existing) => {
            if !same_request(&existing, &req.workflow, &payload) {
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

            state.engine.event_publisher().publish(Event::RunCreated {
                run_id: run.id,
                workflow_name: run.workflow_name.clone(),
                at: Utc::now(),
            });

            Ok((StatusCode::CREATED, ok(RunResponse::from(run))))
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::{Event, EventSubscriber, SubscriberFuture};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::RunStatus;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

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

    fn build_state(counter: Option<Arc<AtomicUsize>>) -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(TestWorkflow).unwrap();
        engine.register(OtherWorkflow).unwrap();
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
        (build_state(Some(counter.clone())), counter)
    }

    fn test_state() -> AppState {
        build_state(None)
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
        let auth_header = make_auth_header(&state);
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
    async fn create_run_unknown_workflow() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
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

    #[tokio::test]
    async fn create_run_without_payload() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
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

    // ---- Idempotency-Key ----

    fn deploy_body() -> JsonValue {
        json!({"workflow": "test-workflow", "payload": {"env": "prod"}})
    }

    #[tokio::test]
    async fn without_header_always_creates_a_new_run() {
        let state = test_state();
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

        let resp = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some("")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn key_at_the_length_limit_is_accepted() {
        let state = test_state();
        let auth = make_auth_header(&state);
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
        let auth = make_auth_header(&state);
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
        let auth = make_auth_header(&state);

        let resp = router(state)
            .oneshot(post_run(&auth, deploy_body(), Some("cle-\u{e9}")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_workflow_does_not_consume_the_key() {
        let state = test_state();
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
        let auth = make_auth_header(&state);

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
}
