//! `POST /api/v1/runs/:id/approve` -- Approve a run awaiting human approval.
//!
//! `POST /api/v1/runs/:id/reject` -- Reject a run awaiting human approval.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate};
use tokio::spawn;
use uuid::Uuid;

use crate::entities::RunResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Approve a run that is awaiting human approval.
///
/// Transitions the run from `AwaitingApproval` back to `Running`.
/// Returns 400 if the run is not in `AwaitingApproval` state.
pub async fn approve_run(
    auth: Authenticated,
    state: State<AppState>,
    path: Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    resolve_approval(auth, state, path, RunStatus::Running, "approve").await
}

/// Reject a run that is awaiting human approval.
///
/// Transitions the run from `AwaitingApproval` to `Failed`.
/// Returns 400 if the run is not in `AwaitingApproval` state.
pub async fn reject_run(
    auth: Authenticated,
    state: State<AppState>,
    path: Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    resolve_approval(auth, state, path, RunStatus::Failed, "reject").await
}

async fn resolve_approval(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    target_status: RunStatus,
    verb: &str,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let run = state.get_run_or_404(id).await?;

    if run.status.state != RunStatus::AwaitingApproval {
        return Err(ApiError::BadRequest(format!(
            "cannot {verb} run in {} state, expected AwaitingApproval",
            run.status.state
        )));
    }

    // On rejection, mark the approval step as Rejected so the dashboard reflects it.
    // On approval, the step is transitioned by the replay in resume_run.
    if target_status == RunStatus::Failed {
        let steps = state.store.list_steps(id).await?;
        for step in &steps {
            if step.status.state == StepStatus::AwaitingApproval {
                state
                    .store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Rejected),
                            completed_at: Some(chrono::Utc::now()),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;
            }
        }
    }

    state.store.update_run_status(id, target_status).await?;

    // On approval, resume the run in the background.
    // The handler is re-executed with step replay: completed steps
    // return cached output, and execution continues from where it
    // stopped.
    if target_status == RunStatus::Running {
        let engine = state.engine.clone();
        spawn(async move {
            if let Err(err) = engine.resume_run(id).await {
                tracing::error!(run_id = %id, error = %err, "failed to resume run after approval");
            }
        });
    }

    let updated = state.get_run_or_404(id).await?;

    Ok(ok(RunResponse::from(updated)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as HttpStatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, NewStep, RunStatus, StepKind, StepStatus, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        let user_store: Arc<dyn ironflow_store::user_store::UserStore> =
            Arc::new(InMemoryStore::new());
        let api_key_store: Arc<dyn ApiKeyStore> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        AppState::new(
            store,
            user_store,
            api_key_store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
        )
    }

    async fn create_awaiting_approval_run(
        store: &Arc<InMemoryStore>,
    ) -> ironflow_store::models::Run {
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::AwaitingApproval)
            .await
            .unwrap();
        store.get_run(run.id).await.unwrap().unwrap()
    }

    // -- approve --

    #[tokio::test]
    async fn approve_awaiting_approval_run() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_awaiting_approval_run(&store).await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/approve", post(approve_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/approve", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["status"], "running");
    }

    #[tokio::test]
    async fn approve_pending_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/approve", post(approve_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/approve", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn approve_nonexistent_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/approve", post(approve_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/approve", Uuid::now_v7()))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::NOT_FOUND);
    }

    // -- reject --

    #[tokio::test]
    async fn reject_awaiting_approval_run() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_awaiting_approval_run(&store).await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["status"], "failed");
    }

    #[tokio::test]
    async fn reject_transitions_approval_step_to_failed() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_awaiting_approval_run(&store).await;

        // Create an approval step in AwaitingApproval state
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                name: "gate".to_string(),
                kind: StepKind::Approval,
                position: 0,
                input: None,
            })
            .await
            .unwrap();
        store
            .update_step(
                step.id,
                ironflow_store::models::StepUpdate {
                    status: Some(StepStatus::Running),
                    ..ironflow_store::models::StepUpdate::default()
                },
            )
            .await
            .unwrap();
        store
            .update_step(
                step.id,
                ironflow_store::models::StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    ..ironflow_store::models::StepUpdate::default()
                },
            )
            .await
            .unwrap();

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        let steps = store.list_steps(run.id).await.unwrap();
        assert_eq!(steps[0].status.state, StepStatus::Rejected);
    }

    #[tokio::test]
    async fn reject_pending_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn reject_nonexistent_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", Uuid::now_v7()))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::NOT_FOUND);
    }
}
