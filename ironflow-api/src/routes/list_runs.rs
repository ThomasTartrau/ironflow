//! `GET /api/v1/runs` — List runs with filtering and pagination.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_store::models::RunFilter;

use crate::entities::{ListRunsQuery, RunResponse};
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// List runs with optional filtering and pagination.
///
/// # Query Parameters
///
/// - `workflow` — Filter by workflow name (optional)
/// - `status` — Filter by run status (optional)
/// - `created_by` — Filter by author user ID (optional). Also matches runs
///   triggered by one of that user's API keys.
/// - `page` — Page number, 1-based (default: 1)
/// - `per_page` — Items per page (default: 20, max: 100)
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/runs",
        tags = ["runs"],
        params(ListRunsQuery),
        responses(
            (status = 200, description = "List of runs with pagination", body = Vec<RunResponse>),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_runs(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ListRunsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let page = params.page.unwrap_or(1);
    let per_page = params.per_page.unwrap_or(20).min(100);

    let labels = params.parse_labels();

    let filter = RunFilter {
        workflow_name: params.workflow,
        status: params.status,
        created_after: None,
        created_before: None,
        has_steps: params.has_steps,
        labels,
        created_by_user_id: params.created_by,
    };

    let page_result = state.store.list_runs(filter, page, per_page).await?;
    let runs: Vec<RunResponse> = page_result
        .items
        .into_iter()
        .map(RunResponse::from)
        .collect();

    Ok(ok_paged(runs, page, per_page, page_result.total))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{
        ApiKeyScope, NewApiKey, NewRun, NewStep, NewUser, RunActor, RunStatus, StepKind,
        TriggerKind,
    };
    use serde_json::{Value as JsonValue, from_slice, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::routes::test_helpers::create_terminal_run;

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

    fn make_auth_header(state: &AppState) -> String {
        use ironflow_auth::jwt::AccessToken;
        use uuid::Uuid;

        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn empty_list() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?page=1&per_page=20")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn with_workflow_filter() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        state
            .store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "deploy".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
            })
            .await
            .unwrap();
        state
            .store
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

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?workflow=deploy")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 1);
        assert_eq!(json_val["data"][0]["workflow_name"], "deploy");
    }

    #[tokio::test]
    async fn with_status_filter() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

        let run = state
            .store
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

        state
            .store
            .update_run_status(run.id, ironflow_store::models::RunStatus::Running)
            .await
            .unwrap();

        // Second run stays Pending
        state
            .store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "other".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
            })
            .await
            .unwrap();

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?status=running")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 1);
        assert_eq!(json_val["data"][0]["status"], "running");
    }

    #[tokio::test]
    async fn pagination_meta_returned() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

        for i in 0..5 {
            state
                .store
                .create_run(NewRun {
                    created_by: None,
                    workflow_name: format!("wf-{i}"),
                    trigger: TriggerKind::Manual,
                    payload: json!({}),
                    max_retries: 0,
                    handler_version: None,
                    labels: HashMap::new(),
                    scheduled_at: None,
                })
                .await
                .unwrap();
        }

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?page=1&per_page=2")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 2);
        assert_eq!(json_val["meta"]["page"], 1);
        assert_eq!(json_val["meta"]["per_page"], 2);
        assert_eq!(json_val["meta"]["total"], 5);
    }

    #[tokio::test]
    async fn per_page_capped_at_100() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?per_page=500")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        // per_page should be capped to 100
        assert_eq!(json_val["meta"]["per_page"], 100);
    }

    #[tokio::test]
    async fn has_steps_true_filters_completed_and_cancelled_empty_runs() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

        let run_with =
            create_terminal_run(state.store.as_ref(), "with-steps", RunStatus::Completed).await;
        let _run_without =
            create_terminal_run(state.store.as_ref(), "without-steps", RunStatus::Completed).await;

        state
            .store
            .create_step(NewStep {
                run_id: run_with.id,
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?has_steps=true")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 1);
        assert_eq!(json_val["data"][0]["workflow_name"], "with-steps");
    }

    #[tokio::test]
    async fn has_steps_false_returns_only_completed_or_cancelled_empty_runs() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

        let run_with =
            create_terminal_run(state.store.as_ref(), "with-steps", RunStatus::Cancelled).await;
        let _run_without =
            create_terminal_run(state.store.as_ref(), "without-steps", RunStatus::Cancelled).await;

        state
            .store
            .create_step(NewStep {
                run_id: run_with.id,
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?has_steps=false")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 1);
        assert_eq!(json_val["data"][0]["workflow_name"], "without-steps");
    }

    #[tokio::test]
    async fn has_steps_true_does_not_hide_pending_runs_without_steps() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

        state
            .store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "pending-no-steps".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
            })
            .await
            .unwrap();

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?has_steps=true")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 1);
        assert_eq!(json_val["data"][0]["workflow_name"], "pending-no-steps");
    }

    // ---- created_by ----

    async fn seed_user(state: &AppState, username: &str) -> Uuid {
        state
            .store
            .create_user(NewUser {
                email: format!("{username}@example.com"),
                username: username.to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .expect("create user")
            .id
    }

    async fn create_run_authored_by(
        state: &AppState,
        workflow: &str,
        created_by: Option<RunActor>,
    ) {
        state
            .store
            .create_run(NewRun {
                workflow_name: workflow.to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by,
            })
            .await
            .expect("create run");
    }

    async fn list(state: AppState, auth_header: String, query: &str) -> (StatusCode, JsonValue) {
        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri(format!("/?{query}"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val = if status == StatusCode::OK {
            from_slice(&body).unwrap()
        } else {
            JsonValue::Null
        };
        (status, json_val)
    }

    #[tokio::test]
    async fn created_by_filter_keeps_only_the_matching_author() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let alice = seed_user(&state, "alice").await;
        let bob = seed_user(&state, "bob").await;

        create_run_authored_by(&state, "by-alice", Some(RunActor::User { user_id: alice })).await;
        create_run_authored_by(&state, "by-bob", Some(RunActor::User { user_id: bob })).await;
        create_run_authored_by(&state, "by-system", None).await;

        let (status, body) = list(state, auth_header, &format!("created_by={alice}")).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
        assert_eq!(body["data"][0]["workflow_name"], "by-alice");
        assert_eq!(body["data"][0]["created_by"]["label"], "alice");
        assert_eq!(body["meta"]["total"], 1);
    }

    #[tokio::test]
    async fn created_by_filter_also_matches_runs_from_the_users_api_keys() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let alice = seed_user(&state, "alice").await;
        let key = state
            .store
            .create_api_key(NewApiKey {
                user_id: alice,
                name: "ci-deploy".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "irfl_0000".to_string(),
                scopes: vec![ApiKeyScope::RunsWrite],
                expires_at: None,
            })
            .await
            .unwrap();

        create_run_authored_by(
            &state,
            "by-alice-key",
            Some(RunActor::ApiKey {
                api_key_id: key.id,
                user_id: alice,
            }),
        )
        .await;

        let (_, body) = list(state, auth_header, &format!("created_by={alice}")).await;

        assert_eq!(body["data"].as_array().unwrap().len(), 1);
        assert_eq!(body["data"][0]["created_by"]["kind"], "api_key");
        assert_eq!(body["data"][0]["created_by"]["id"], key.id.to_string());
        assert_eq!(body["data"][0]["created_by"]["label"], "ci-deploy (alice)");
    }

    #[tokio::test]
    async fn created_by_filter_with_an_unknown_author_returns_an_empty_page() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        create_run_authored_by(&state, "by-system", None).await;

        let (status, body) = list(
            state,
            auth_header,
            &format!("created_by={}", Uuid::now_v7()),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].as_array().unwrap().is_empty());
        assert_eq!(body["meta"]["total"], 0);
    }

    #[tokio::test]
    async fn created_by_filter_rejects_a_non_uuid_value() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

        let (status, _) = list(state, auth_header, "created_by=not-a-uuid").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_run_without_an_author_is_listed_as_system() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        create_run_authored_by(&state, "legacy", None).await;

        let (status, body) = list(state, auth_header, "").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"][0]["created_by"]["kind"], "system");
        assert!(body["data"][0]["created_by"]["id"].is_null());
        assert_eq!(body["data"][0]["created_by"]["label"], "api");
    }
}
