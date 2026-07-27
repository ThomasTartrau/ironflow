//! `POST /api/v1/runs/:id/retry` — Retry a failed run.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::Event;
use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
use uuid::Uuid;

use crate::entities::RunResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Retry a failed run.
///
/// Creates a new `Pending` run with `TriggerKind::Retry` pointing to the
/// original. Returns 400 if the run is not in a retryable state, and 409 if an
/// automatic retry is already armed for it.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/retry",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 201, description = "Run retry created successfully", body = RunResponse),
            (status = 400, description = "Run cannot be retried"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run not found"),
            (status = 409, description = "Run is already waiting for an automatic retry")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn retry_run(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let original = state.get_run_or_404(id).await?;

    // A `Retrying` run already has an automatic retry armed: creating a second
    // run here would execute the same work twice, concurrently.
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

    let new_run = state
        .store
        .create_run(NewRun {
            workflow_name: original.workflow_name,
            trigger: TriggerKind::Retry { parent_run_id: id },
            payload: original.payload,
            max_retries: original.max_retries,
            handler_version: original.handler_version,
            labels: original.labels,
            scheduled_at: None,
            // The retry inherits the original cap, so a run cancelled for
            // reaching it does not silently come back unbounded.
            max_cost_usd: original.max_cost_usd,
        })
        .await?;

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
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
    use ironflow_store::store::RunStore;
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
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({"key": "value"}),
                max_retries: 3,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

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
                max_cost_usd: Some(cap),
            })
            .await
            .unwrap();

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
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
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
    async fn retry_completed_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

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
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

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
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 3,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

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
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

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
}
