//! `GET /api/v1/steps` -- List the agent steps of a role across runs and workflows.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::config::MAX_ROLE_NAME_LEN;
use ironflow_store::entities::ApiKeyScope;

use crate::authorization::AuthorizeExt;
use crate::entities::{ListStepsQuery, StepResponse};
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// List the agent steps of one agent role, across every run and workflow.
///
/// # Query Parameters
///
/// - `agent_role` - Name of the `AgentRole`, matched exactly (required, at
///   most 128 bytes)
/// - `page` - Page number, 1-based (default: 1)
/// - `per_page` - Items per page (default: 20, max: 100)
///
/// Steps are sorted newest first. Steps stored before roles were recorded have
/// no role and are never listed.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/steps",
        tags = ["runs"],
        params(ListStepsQuery),
        responses(
            (status = 200, description = "Agent steps of the role with pagination", body = Vec<StepResponse>),
            (status = 400, description = "Missing, blank or too long agent_role"),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_steps(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ListStepsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth.require(ApiKeyScope::RunsRead, false)?;
    let agent_role = params.agent_role.trim();
    if agent_role.is_empty() {
        return Err(ApiError::BadRequest(
            "agent_role must not be empty".to_string(),
        ));
    }
    if agent_role.len() > MAX_ROLE_NAME_LEN {
        return Err(ApiError::BadRequest(format!(
            "agent_role must be at most {MAX_ROLE_NAME_LEN} bytes"
        )));
    }
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);

    let page_result = state
        .store
        .list_steps_by_agent_role(agent_role, page, per_page)
        .await?;
    let steps: Vec<StepResponse> = page_result
        .items
        .into_iter()
        .map(StepResponse::from)
        .collect();

    Ok(ok_paged(steps, page, per_page, page_result.total))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewStep, StepKind, step_trace_id};
    use ironflow_store::store::RunStore;
    use serde_json::{Value as JsonValue, from_slice, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::routes::test_helpers::{
        create_named_run, create_user_auth_header, state_with_engine,
    };

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
        state_with_engine(store, engine, false)
    }

    async fn create_run(store: &InMemoryStore, workflow: &str) -> Uuid {
        create_named_run(store, workflow).await.id
    }

    async fn create_agent_step(store: &InMemoryStore, run_id: Uuid, role: Option<&str>) -> Uuid {
        let input = match role {
            Some(role) => json!({"type": "agent", "prompt": "p", "role": role}),
            None => json!({"type": "agent", "prompt": "p"}),
        };
        store
            .create_step(NewStep {
                run_id,
                trace_id: step_trace_id(run_id, "review", 0),
                name: "review".to_string(),
                kind: StepKind::Agent,
                position: 0,
                input: Some(input),
                is_error_handler: false,
            })
            .await
            .expect("create step")
            .id
    }

    async fn fetch(store: Arc<InMemoryStore>, uri: &str) -> (StatusCode, JsonValue) {
        let state = test_state(store);
        let auth_header = create_user_auth_header(&state, "testuser", false).await;
        let app = Router::new().route("/", get(list_steps)).with_state(state);

        let req = Request::builder()
            .uri(uri)
            .header("authorization", auth_header)
            .body(Body::empty())
            .expect("request");
        let resp = app.oneshot(req).await.expect("response");
        let status = resp.status();
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let json_val: JsonValue = if body.is_empty() {
            JsonValue::Null
        } else {
            from_slice(&body).unwrap_or(JsonValue::Null)
        };
        (status, json_val)
    }

    #[tokio::test]
    async fn list_steps_by_agent_role_returns_steps_of_every_run() {
        let store = Arc::new(InMemoryStore::new());
        let deploy = create_run(&store, "deploy").await;
        let release = create_run(&store, "release").await;
        let first = create_agent_step(&store, deploy, Some("Reviewer")).await;
        let second = create_agent_step(&store, release, Some("Reviewer")).await;
        create_agent_step(&store, release, Some("Writer")).await;
        create_agent_step(&store, deploy, None).await;

        let (status, json_val) = fetch(store, "/?agent_role=Reviewer").await;
        assert_eq!(status, StatusCode::OK);

        let items = json_val["data"].as_array().expect("data array");
        let ids: Vec<&str> = items
            .iter()
            .map(|s| s["id"].as_str().expect("id"))
            .collect();
        assert_eq!(ids, vec![second.to_string(), first.to_string()]);
        assert!(items.iter().all(|s| s["agent_role"] == "Reviewer"));
        assert_eq!(json_val["meta"]["total"], 2);
        assert_eq!(json_val["meta"]["page"], 1);
        assert_eq!(json_val["meta"]["per_page"], 20);
    }

    #[tokio::test]
    async fn list_steps_clamps_the_page_and_page_size() {
        let store = Arc::new(InMemoryStore::new());
        let run_id = create_run(&store, "deploy").await;
        create_agent_step(&store, run_id, Some("Reviewer")).await;

        let (status, json_val) = fetch(store, "/?agent_role=Reviewer&page=0&per_page=500").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json_val["meta"]["page"], 1);
        assert_eq!(json_val["meta"]["per_page"], 100);
        assert_eq!(json_val["data"].as_array().expect("data array").len(), 1);
    }

    #[tokio::test]
    async fn list_steps_of_an_unknown_role_is_empty() {
        let store = Arc::new(InMemoryStore::new());
        let (status, json_val) = fetch(store, "/?agent_role=Nobody").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json_val["data"].as_array().expect("data array").is_empty());
        assert_eq!(json_val["meta"]["total"], 0);
    }

    #[tokio::test]
    async fn list_steps_without_agent_role_is_rejected() {
        let (status, _) = fetch(Arc::new(InMemoryStore::new()), "/").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_steps_with_blank_agent_role_is_rejected() {
        let store = Arc::new(InMemoryStore::new());
        let (status, _) = fetch(store.clone(), "/?agent_role=").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = fetch(store, "/?agent_role=%20%20").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_steps_with_too_long_agent_role_is_rejected() {
        let uri = format!("/?agent_role={}", "r".repeat(MAX_ROLE_NAME_LEN + 1));
        let (status, _) = fetch(Arc::new(InMemoryStore::new()), &uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_steps_requires_auth() {
        let state = test_state(Arc::new(InMemoryStore::new()));
        let app = Router::new().route("/", get(list_steps)).with_state(state);
        let req = Request::builder()
            .uri("/?agent_role=Reviewer")
            .body(Body::empty())
            .expect("request");
        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
