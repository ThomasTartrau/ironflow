//! `POST /api/v1/runs/:id/retry` — Retry a failed run.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::Event;
use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
use uuid::Uuid;

use crate::actor::run_actor_of;
use crate::entities::RunResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Retry a failed run.
///
/// Creates a new `Pending` run with `TriggerKind::Retry` pointing to the
/// original. Returns 400 if the run is not in a retryable state.
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
            (status = 404, description = "Run not found")
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

    if !matches!(
        original.status.state,
        RunStatus::Failed | RunStatus::Retrying | RunStatus::Cancelled
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
            // A retry is a new, accountable action: the author is whoever
            // triggered it, not the author of the original run.
            created_by: Some(run_actor_of(&auth)),
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
    use ironflow_store::models::{NewRun, NewUser, RunActor, RunStatus, TriggerKind};
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
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
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
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
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
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
}
