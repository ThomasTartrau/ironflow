//! `GET /api/v1/runs/:id/logs` -- Retrieve persisted log lines for a run.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
#[cfg(feature = "openapi")]
use ironflow_store::entities::LogEntry;
use ironflow_store::entities::{LogFilter, LogStream};
use ironflow_types::{ApiMeta, ApiResponse};

use crate::error::ApiError;
use crate::state::AppState;

const MAX_LIMIT: u32 = 1000;
const DEFAULT_LIMIT: u32 = 100;

/// Query parameters for listing run logs.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
pub struct GetRunLogsQuery {
    /// Filter by step ID.
    pub step_id: Option<Uuid>,
    /// Filter by output stream (`stdout`, `stderr`, `system`).
    pub stream: Option<LogStream>,
    /// Cursor for pagination (last entry ID from previous page).
    pub cursor: Option<Uuid>,
    /// Number of entries to return (default: 100, max: 1000).
    pub limit: Option<u32>,
}

/// Cursor-based pagination metadata for log entries.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogCursorMeta {
    /// Cursor to pass for the next page. `None` when there are no more entries.
    pub next_cursor: Option<Uuid>,
    /// Whether more entries exist after this page.
    pub has_more: bool,
}

/// Retrieve persisted log lines for a run with cursor-based pagination.
///
/// Returns log entries ordered by time (UUID v7 ascending). Use the
/// `cursor` query parameter with the last entry's `id` to fetch the
/// next page.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/runs/{id}/logs",
        tags = ["runs"],
        params(
            ("id" = Uuid, Path, description = "Run ID"),
            GetRunLogsQuery,
        ),
        responses(
            (status = 200, description = "Log entries with cursor-based pagination", body = Vec<LogEntry>),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_run_logs(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
    Query(params): Query<GetRunLogsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    state.get_run_or_404(run_id).await?;

    let limit = match params.limit {
        Some(0) | None => DEFAULT_LIMIT,
        Some(l) => l.min(MAX_LIMIT),
    };

    let filter = LogFilter {
        step_id: params.step_id,
        stream: params.stream,
    };

    let entries = state
        .store
        .get_logs(run_id, filter, params.cursor, limit + 1)
        .await?;

    let has_more = entries.len() > limit as usize;
    let entries: Vec<_> = entries.into_iter().take(limit as usize).collect();
    let next_cursor = if has_more {
        entries.last().map(|e| e.id)
    } else {
        None
    };

    let cursor_meta = LogCursorMeta {
        next_cursor,
        has_more,
    };
    let extra = match serde_json::to_value(cursor_meta) {
        Ok(Value::Object(map)) => map.into_iter().collect(),
        _ => HashMap::new(),
    };

    Ok(Json(ApiResponse {
        data: entries,
        meta: Some(ApiMeta {
            page: None,
            per_page: None,
            total: None,
            extra,
        }),
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use serde_json::{Value as JsonValue, from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{LogStream, NewLogEntries, NewRun, TriggerKind};
    use ironflow_store::memory::InMemoryStore;

    use super::*;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(JwtConfig {
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
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    async fn create_run(state: &AppState) -> Uuid {
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
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run()
            .id
    }

    async fn push_logs(state: &AppState, run_id: Uuid, step_id: Uuid, stream: LogStream, n: usize) {
        state
            .store
            .append_logs(NewLogEntries {
                run_id,
                step_id,
                step_name: "build".to_string(),
                stream,
                lines: (0..n).map(|i| format!("line {i}")).collect(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn returns_persisted_logs() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let run_id = create_run(&state).await;
        let step_id = Uuid::now_v7();

        push_logs(&state, run_id, step_id, LogStream::Stdout, 3).await;

        let app = Router::new()
            .route("/runs/{id}/logs", get(get_run_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 3);
        assert_eq!(json_val["data"][0]["line"], "line 0");
        assert_eq!(json_val["data"][0]["stream"], "stdout");
        assert_eq!(json_val["meta"]["has_more"], false);
        assert!(json_val["meta"]["next_cursor"].is_null());
    }

    #[tokio::test]
    async fn filters_by_step_id() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let run_id = create_run(&state).await;
        let step_a = Uuid::now_v7();
        let step_b = Uuid::now_v7();

        push_logs(&state, run_id, step_a, LogStream::Stdout, 2).await;
        push_logs(&state, run_id, step_b, LogStream::Stdout, 3).await;

        let app = Router::new()
            .route("/runs/{id}/logs", get(get_run_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs?step_id={step_a}"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn filters_by_stream() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let run_id = create_run(&state).await;
        let step_id = Uuid::now_v7();

        push_logs(&state, run_id, step_id, LogStream::Stdout, 2).await;
        push_logs(&state, run_id, step_id, LogStream::Stderr, 1).await;

        let app = Router::new()
            .route("/runs/{id}/logs", get(get_run_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs?stream=stderr"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 1);
        assert_eq!(json_val["data"][0]["stream"], "stderr");
    }

    #[tokio::test]
    async fn cursor_based_pagination() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let run_id = create_run(&state).await;
        let step_id = Uuid::now_v7();

        push_logs(&state, run_id, step_id, LogStream::Stdout, 5).await;

        let app = Router::new()
            .route("/runs/{id}/logs", get(get_run_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs?limit=2"))
            .header("authorization", &auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let page1: JsonValue = from_slice(&body).unwrap();
        assert_eq!(page1["data"].as_array().unwrap().len(), 2);
        assert_eq!(page1["meta"]["has_more"], true);

        let cursor = page1["meta"]["next_cursor"].as_str().unwrap();

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs?limit=2&cursor={cursor}"))
            .header("authorization", &auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let page2: JsonValue = from_slice(&body).unwrap();
        assert_eq!(page2["data"].as_array().unwrap().len(), 2);
        assert_eq!(page2["data"][0]["line"], "line 2");
        assert_eq!(page2["meta"]["has_more"], true);

        let cursor = page2["meta"]["next_cursor"].as_str().unwrap();

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs?limit=2&cursor={cursor}"))
            .header("authorization", &auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let page3: JsonValue = from_slice(&body).unwrap();
        assert_eq!(page3["data"].as_array().unwrap().len(), 1);
        assert_eq!(page3["meta"]["has_more"], false);
        assert!(page3["meta"]["next_cursor"].is_null());
    }

    #[tokio::test]
    async fn run_not_found_returns_404() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/runs/{id}/logs", get(get_run_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/runs/{}/logs", Uuid::now_v7()))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unauthenticated_returns_401() {
        let state = test_state();
        let run_id = create_run(&state).await;
        let app = Router::new()
            .route("/runs/{id}/logs", get(get_run_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn limit_capped_at_1000() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let run_id = create_run(&state).await;
        let step_id = Uuid::now_v7();

        push_logs(&state, run_id, step_id, LogStream::Stdout, 2).await;

        let app = Router::new()
            .route("/runs/{id}/logs", get(get_run_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs?limit=5000"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn empty_logs_returns_empty_array() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let run_id = create_run(&state).await;

        let app = Router::new()
            .route("/runs/{id}/logs", get(get_run_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/runs/{run_id}/logs"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 0);
        assert_eq!(json_val["meta"]["has_more"], false);
    }
}
