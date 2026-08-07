//! `POST /api/v1/runs/:id/retry` — Retry a failed run.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::error::HANDLER_VERSION_MISMATCH_CODE;
use ironflow_engine::notify::Event;
use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
use serde::Deserialize;
use uuid::Uuid;

use crate::actor::run_actor_of;
use crate::entities::RunResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Query parameters for `POST /api/v1/runs/:id/retry`.
#[derive(Debug, Deserialize, Default)]
pub struct RetryQuery {
    /// Force the retry even when the handler version has changed.
    #[serde(default)]
    pub force: bool,
}

/// Retry a failed run.
///
/// Creates a new `Pending` run with `TriggerKind::Retry` pointing to the
/// original. Returns 400 if the run is not in a retryable state, 409 if an
/// automatic retry is already armed, and 409 if the handler version has
/// changed since the original run (pass `?force=true` to override).
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/retry",
        tags = ["runs"],
        params(
            ("id" = Uuid, Path, description = "Run ID"),
            ("force" = bool, Query, description = "Force retry despite handler version mismatch"),
        ),
        responses(
            (status = 201, description = "Run retry created successfully", body = RunResponse),
            (status = 400, description = "Run cannot be retried"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run not found"),
            (status = 409, description = "Version mismatch or automatic retry already armed")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn retry_run(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<RetryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let original = state.get_run_or_404(id).await?;

    // Retrying means an automatic retry is already armed; creating a manual
    // retry on top would produce a duplicate run once the timer fires.
    if original.status.state == RunStatus::Retrying {
        return Err(ApiError::Conflict(
            "run is already waiting for an automatic retry; cancel it first to retry manually"
                .to_string(),
        ));
    }

    if !matches!(
        original.status.state,
        RunStatus::Failed | RunStatus::Cancelled
    ) {
        return Err(ApiError::BadRequest(format!(
            "cannot retry run in {} state",
            original.status.state
        )));
    }

    let force = query.force;
    let handler = state.engine.get_handler(&original.workflow_name);
    let current_version = handler.and_then(|h| h.version().map(str::to_string));

    if let Some(handler) = &handler
        && !handler.is_version_compatible(original.handler_version.as_deref())
        && !force
    {
        return Err(ApiError::Conflict(format!(
            "{}: handler '{}' is now at version {}, but the run was created \
             with version {}. Pass ?force=true to override.",
            HANDLER_VERSION_MISMATCH_CODE,
            original.workflow_name,
            current_version.as_deref().unwrap_or("unknown"),
            original.handler_version.as_deref().unwrap_or("unknown"),
        )));
    }

    // When the handler is still registered, use its current version.
    // When unregistered (e.g. removed between deploys), preserve the
    // original version so version-tracking information is not lost.
    let effective_version = current_version.clone().or(original.handler_version.clone());

    let new_run = state
        .store
        .create_run(NewRun {
            workflow_name: original.workflow_name.clone(),
            trigger: TriggerKind::Retry { parent_run_id: id },
            payload: original.payload,
            max_retries: original.max_retries,
            handler_version: effective_version,
            labels: original.labels,
            scheduled_at: None,
            // The retry is attributed to the user who triggered it, not the
            // original author, so the audit trail shows who actually acted.
            created_by: Some(run_actor_of(&auth)),
            // A retry must not inherit the parent's idempotency key: it is a
            // new logical operation and must be eligible for its own dedup.
            idempotency_key: None,
            // Inherit the original cost cap so budget constraints survive retries.
            max_cost_usd: original.max_cost_usd,
        })
        .await?
        .into_run();

    // Only emit RetryForced when both the handler is registered (so
    // current_version is meaningful) and the versions actually differ.
    if force && handler.is_some() && original.handler_version != current_version {
        state.engine.event_publisher().publish(Event::RetryForced {
            run_id: new_run.id,
            workflow_name: original.workflow_name.clone(),
            original_version: original.handler_version.unwrap_or_default(),
            current_version: current_version.unwrap_or_default(),
            at: Utc::now(),
        });
    }

    state.engine.event_publisher().publish(Event::RunCreated {
        run_id: new_run.id,
        workflow_name: new_run.workflow_name.clone(),
        at: Utc::now(),
    });

    Ok((StatusCode::CREATED, ok(RunResponse::from(new_run))))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as HttpStatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, NewUser, RunActor, RunStatus, TriggerKind};
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
    use rust_decimal::Decimal;
    use serde_json::{Value as JsonValue, from_slice, from_value, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    #[tokio::test]
    async fn retry_failed_run() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({"key": "value"}),
                max_retries: 3,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Failed)
            .await
            .unwrap();

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let new_id: Uuid = from_value(json_val["data"]["id"].clone()).unwrap();

        let new_run = store.get_run(new_id).await.unwrap().unwrap();
        assert_eq!(new_run.status.state, RunStatus::Pending);
        assert!(matches!(new_run.trigger, TriggerKind::Retry { .. }));
    }

    #[tokio::test]
    async fn retry_inherits_the_original_cost_cap() {
        let store = Arc::new(InMemoryStore::new());
        let cap = Decimal::new(250, 2);
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 1,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: None,
                max_cost_usd: Some(cap),
            })
            .await
            .unwrap()
            .into_run();

        // A run cancelled for reaching its cap is retryable.
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Cancelled)
            .await
            .unwrap();

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let new_id: Uuid = from_value(json_val["data"]["id"].clone()).unwrap();

        let new_run = store.get_run(new_id).await.unwrap().unwrap();
        assert_eq!(new_run.max_cost_usd, Some(cap));
    }

    #[tokio::test]
    async fn retry_pending_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn retry_completed_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Completed)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn retry_running_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn retry_run_awaiting_automatic_retry_returns_409() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 3,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Retrying)
            .await
            .unwrap();

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CONFLICT);

        // No duplicate run was created.
        let runs = store
            .list_runs(ironflow_store::models::RunFilter::default(), 1, 50)
            .await
            .unwrap();
        assert_eq!(runs.total, 1);
    }

    #[tokio::test]
    async fn retry_cancelled_run_is_allowed() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Cancelled)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);
    }

    #[tokio::test]
    async fn retry_nonexistent_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", Uuid::now_v7()))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::NOT_FOUND);
    }

    // ---- created_by ----

    #[tokio::test]
    async fn retry_attributes_the_new_run_to_the_caller_not_the_original_author() {
        let store = Arc::new(InMemoryStore::new());
        let original_author = store
            .create_user(NewUser {
                email: "alice@example.com".to_string(),
                username: "alice".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(true),
            })
            .await
            .unwrap();
        let retrying_user = store
            .create_user(NewUser {
                email: "bob@example.com".to_string(),
                username: "bob".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(true),
            })
            .await
            .unwrap();

        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 3,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: Some(RunActor::User {
                    user_id: original_author.id,
                }),
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Failed)
            .await
            .unwrap();

        let state = test_state(store.clone());
        let token =
            AccessToken::for_user(retrying_user.id, "bob", true, &state.jwt_config).unwrap();
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", token.0))
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["created_by"]["kind"], "user");
        assert_eq!(
            json_val["data"]["created_by"]["id"],
            retrying_user.id.to_string()
        );
        assert_eq!(json_val["data"]["created_by"]["label"], "bob");
    }

    // ---- Idempotency-Key ----

    #[tokio::test]
    async fn retry_does_not_inherit_the_idempotency_key() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Api,
                payload: json!({"key": "value"}),
                max_retries: 3,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: Some("github:abc-123".to_string()),
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Failed)
            .await
            .unwrap();

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert!(
            json_val["data"].get("idempotency_key").is_none(),
            "the retry run must not carry the original key"
        );

        // The original key still resolves to the original run.
        let bound = store
            .find_run_by_idempotency_key("github:abc-123")
            .await
            .unwrap()
            .expect("key still bound");
        assert_eq!(bound.id, run.id);
    }

    // ---- Handler version compatibility ----

    struct V2Handler;
    impl WorkflowHandler for V2Handler {
        fn name(&self) -> &str {
            "versioned-wf"
        }
        fn version(&self) -> Option<&str> {
            Some("2.0.0")
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    struct V2CompatHandler;
    impl WorkflowHandler for V2CompatHandler {
        fn name(&self) -> &str {
            "compat-wf"
        }
        fn version(&self) -> Option<&str> {
            Some("2.0.0")
        }
        fn compatible_versions(&self) -> &[&str] {
            &["1.0.0"]
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    fn test_state_with_handlers(store: Arc<InMemoryStore>) -> AppState {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(V2Handler).unwrap();
        engine.register(V2CompatHandler).unwrap();
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    async fn create_failed_run(
        store: &Arc<InMemoryStore>,
        workflow_name: &str,
        handler_version: Option<&str>,
    ) -> ironflow_store::models::Run {
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: workflow_name.to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: handler_version.map(str::to_string),
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Failed)
            .await
            .unwrap();
        run
    }

    #[tokio::test]
    async fn retry_same_version_succeeds() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_failed_run(&store, "versioned-wf", Some("2.0.0")).await;

        let state = test_state_with_handlers(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);
    }

    #[tokio::test]
    async fn retry_version_mismatch_returns_409() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_failed_run(&store, "versioned-wf", Some("1.0.0")).await;

        let state = test_state_with_handlers(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CONFLICT);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let msg = json_val["error"]["message"].as_str().unwrap();
        assert!(msg.contains("HANDLER_VERSION_MISMATCH"));
        assert!(msg.contains("2.0.0"));
        assert!(msg.contains("1.0.0"));
    }

    #[tokio::test]
    async fn retry_version_mismatch_with_force_succeeds() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_failed_run(&store, "versioned-wf", Some("1.0.0")).await;

        let state = test_state_with_handlers(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry?force=true", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let new_id: Uuid = from_value(json_val["data"]["id"].clone()).unwrap();
        let new_run = store.get_run(new_id).await.unwrap().unwrap();
        assert_eq!(new_run.handler_version, Some("2.0.0".to_string()));
    }

    #[tokio::test]
    async fn retry_compatible_version_succeeds() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_failed_run(&store, "compat-wf", Some("1.0.0")).await;

        let state = test_state_with_handlers(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);
    }

    #[tokio::test]
    async fn retry_null_version_is_compatible() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_failed_run(&store, "versioned-wf", None).await;

        let state = test_state_with_handlers(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);
    }

    #[tokio::test]
    async fn retry_writes_current_handler_version() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_failed_run(&store, "versioned-wf", Some("2.0.0")).await;

        let state = test_state_with_handlers(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/retry", post(retry_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/retry", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let new_id: Uuid = from_value(json_val["data"]["id"].clone()).unwrap();
        let new_run = store.get_run(new_id).await.unwrap().unwrap();
        assert_eq!(new_run.handler_version, Some("2.0.0".to_string()));
    }
}
