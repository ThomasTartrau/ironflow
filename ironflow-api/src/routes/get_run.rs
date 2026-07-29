//! `GET /api/v1/runs/:id` — Get run details with steps.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use tokio::join;
use uuid::Uuid;

use crate::entities::{ArtifactResponse, RunDetailResponse, RunResponse, StepResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get a run by ID, including all its steps and dependency edges.
///
/// Returns 404 if the run does not exist.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/runs/{id}",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 200, description = "Run details with steps", body = RunDetailResponse),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_run(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let run = state.get_run_or_404(id).await?;

    // Artifacts are fetched for the whole run in one call and grouped by step,
    // exactly like the dependency edges: one query, never one per step.
    let (steps, deps, artifacts) = join!(
        state.store.list_steps(id),
        state.store.list_step_dependencies(id),
        state.store.list_artifacts_for_run(id)
    );
    let steps = steps?;
    let deps = deps?;
    let artifacts = artifacts?;

    let mut deps_map: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for dep in &deps {
        deps_map
            .entry(dep.step_id)
            .or_default()
            .push(dep.depends_on);
    }

    let mut artifacts_map: HashMap<Uuid, Vec<ArtifactResponse>> = HashMap::new();
    for artifact in artifacts {
        artifacts_map
            .entry(artifact.step_id)
            .or_default()
            .push(ArtifactResponse::from(artifact));
    }

    let step_responses: Vec<StepResponse> = steps
        .into_iter()
        .map(|step| {
            let step_deps = deps_map.remove(&step.id).unwrap_or_default();
            let step_artifacts = artifacts_map.remove(&step.id).unwrap_or_default();
            StepResponse::with_dependencies_and_artifacts(step, step_deps, step_artifacts)
        })
        .collect();

    let payload = run.payload.clone();
    let response = RunDetailResponse {
        run: RunResponse::from(run),
        steps: step_responses,
        payload,
    };

    Ok(ok(response))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, NewUser, RunActor, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
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
    async fn existing_run() {
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

        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        Arc::new(InMemoryStore::new());
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        );
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/{id}", get(get_run)).with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", run.id))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["run"]["id"], run.id.to_string());
    }

    #[tokio::test]
    async fn not_found() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/{id}", get(get_run)).with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", Uuid::nil()))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn detail_exposes_the_run_author() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let user = state
            .store
            .create_user(NewUser {
                email: "alice@example.com".to_string(),
                username: "alice".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();

        let run = state
            .store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                created_by: Some(RunActor::User { user_id: user.id }),
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let app = Router::new().route("/{id}", get(get_run)).with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", run.id))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["run"]["created_by"]["kind"], "user");
        assert_eq!(
            json_val["data"]["run"]["created_by"]["id"],
            user.id.to_string()
        );
        assert_eq!(json_val["data"]["run"]["created_by"]["label"], "alice");
    }
}
