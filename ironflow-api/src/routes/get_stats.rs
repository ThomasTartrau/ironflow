//! `GET /api/v1/stats` — Aggregate statistics across runs.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_store::models::RunFilter;

use crate::entities::{ListRunsQuery, StatsResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get aggregate statistics across runs matching the filter.
///
/// Accepts the same filtering query parameters as `GET /api/v1/runs`
/// (`workflow`, `status`, `has_steps`, `label`, `created_by`). `page` and
/// `per_page` are ignored.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/stats",
        tags = ["stats"],
        params(ListRunsQuery),
        responses(
            (status = 200, description = "Aggregate statistics", body = StatsResponse),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_stats(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ListRunsQuery>,
) -> Result<impl IntoResponse, ApiError> {
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
    let stats = state.store.get_stats(filter).await?;

    let success_rate_percent = if stats.completed_runs + stats.failed_runs > 0 {
        (stats.completed_runs as f64 / (stats.completed_runs + stats.failed_runs) as f64) * 100.0
    } else {
        0.0
    };

    Ok(ok(StatsResponse {
        total_runs: stats.total_runs,
        completed_runs: stats.completed_runs,
        failed_runs: stats.failed_runs,
        cancelled_runs: stats.cancelled_runs,
        active_runs: stats.active_runs,
        success_rate_percent,
        total_cost_usd: stats.total_cost_usd,
        total_duration_ms: stats.total_duration_ms,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
    use ironflow_store::models::{
        NewRun, NewStep, NewUser, RunActor, RunStatus, StepKind, TriggerKind,
    };
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
    use serde_json::{Value as JsonValue, from_slice, json};
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
    async fn empty_stats() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["total_runs"], 0);
    }

    #[tokio::test]
    async fn stats_with_mixed_runs() {
        let store = Arc::new(InMemoryStore::new());

        // Pending
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "a".to_string(),
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

        // Completed
        let r2 = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "b".to_string(),
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
            .update_run_status(r2.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r2.id, RunStatus::Completed)
            .await
            .unwrap();

        // Failed
        let r3 = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "c".to_string(),
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
            .update_run_status(r3.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r3.id, RunStatus::Failed)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["total_runs"], 3);
        assert_eq!(json_val["data"]["completed_runs"], 1);
        assert_eq!(json_val["data"]["failed_runs"], 1);
        assert_eq!(json_val["data"]["active_runs"], 1);

        let rate = json_val["data"]["success_rate_percent"].as_f64().unwrap();
        assert!((rate - 50.0).abs() < 0.01);
    }

    use crate::routes::test_helpers::create_terminal_run;

    async fn setup_runs_with_steps(store: &Arc<InMemoryStore>) {
        let _empty = create_terminal_run(store.as_ref(), "empty-wf", RunStatus::Completed).await;
        let r = create_terminal_run(store.as_ref(), "busy-wf", RunStatus::Completed).await;
        store
            .create_step(NewStep {
                run_id: r.id,
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn stats_filters_has_steps_true() {
        let store = Arc::new(InMemoryStore::new());
        setup_runs_with_steps(&store).await;

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri("/?has_steps=true")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["total_runs"], 1);
    }

    #[tokio::test]
    async fn stats_filters_has_steps_false() {
        let store = Arc::new(InMemoryStore::new());
        setup_runs_with_steps(&store).await;

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri("/?has_steps=false")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["total_runs"], 1);
    }

    #[tokio::test]
    async fn stats_filters_by_workflow_name() {
        let store = Arc::new(InMemoryStore::new());
        setup_runs_with_steps(&store).await;

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri("/?workflow=busy")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["total_runs"], 1);
    }

    #[tokio::test]
    async fn stats_filters_by_status() {
        let store = Arc::new(InMemoryStore::new());
        // One pending, one completed
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "a".to_string(),
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
        let r = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "b".to_string(),
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
            .update_run_status(r.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r.id, RunStatus::Completed)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri("/?status=completed")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["total_runs"], 1);
        assert_eq!(json_val["data"]["completed_runs"], 1);
    }

    #[tokio::test]
    async fn stats_with_created_by_filter() {
        let store = Arc::new(InMemoryStore::new());
        let alice = store
            .create_user(NewUser {
                email: "alice@example.com".to_string(),
                username: "alice".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();
        let bob = store
            .create_user(NewUser {
                email: "bob@example.com".to_string(),
                username: "bob".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();

        for user_id in [alice.id, bob.id] {
            store
                .create_run(NewRun {
                    workflow_name: "test".to_string(),
                    trigger: TriggerKind::Api,
                    payload: json!({}),
                    max_retries: 0,
                    handler_version: None,
                    labels: HashMap::new(),
                    scheduled_at: None,
                    created_by: Some(RunActor::User { user_id }),
                    idempotency_key: None,
                    max_cost_usd: None,
                })
                .await
                .unwrap();
        }

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri(format!("/?created_by={}", alice.id))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["total_runs"], 1);
    }
}
