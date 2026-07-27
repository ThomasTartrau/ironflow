//! `POST /api/v1/internal/runs/:id/logs` -- Push log lines for real-time streaming.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use serde_json::json;

use ironflow_engine::notify::{Event, LogStream};

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

const MAX_LINES_PER_BATCH: usize = 1000;

/// Request body for pushing log lines from the worker.
#[derive(Debug, Serialize, Deserialize)]
pub struct PushLogsRequest {
    /// Step that produced the log lines.
    pub step_id: Uuid,
    /// Human-readable step name.
    pub step_name: String,
    /// Output stream.
    pub stream: LogStream,
    /// Log lines to broadcast.
    pub lines: Vec<String>,
}

/// Push log lines for a running step.
///
/// Each line is broadcast as an [`Event::LogLine`] so SSE clients can
/// stream step output in real time without waiting for completion.
pub async fn push_logs(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
    Json(req): Json<PushLogsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.lines.len() > MAX_LINES_PER_BATCH {
        return Err(ApiError::BadRequest(format!(
            "too many lines: {} (max {MAX_LINES_PER_BATCH})",
            req.lines.len()
        )));
    }

    state
        .store
        .get_run(run_id)
        .await?
        .ok_or(ApiError::RunNotFound(run_id))?;

    let now = Utc::now();

    for line in &req.lines {
        let event = Event::LogLine {
            run_id,
            step_id: req.step_id,
            step_name: req.step_name.clone(),
            stream: req.stream,
            line: line.clone(),
            at: now,
        };
        let _ = state.event_sender.send(event);
    }

    Ok(ok(json!({ "accepted": req.lines.len() })))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{Value as JsonValue, from_slice, json, to_string};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::broadcast;
    use tokio::time::timeout;
    use tower::ServiceExt;
    use uuid::Uuid;

    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::{Event, LogStream};
    use ironflow_store::entities::{NewStep, StepKind};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};

    use super::PushLogsRequest;
    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(64);
        AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    #[tokio::test]
    async fn push_logs_broadcasts_events() {
        timeout(Duration::from_secs(10), async {
            let state = test_state();
            let run = state
                .store
                .create_run(NewRun {
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

            let step = state
                .store
                .create_step(NewStep {
                    run_id: run.id,
                    name: "build".to_string(),
                    kind: StepKind::Shell,
                    position: 0,
                    input: None,
                })
                .await
                .unwrap();

            let mut rx = state.event_sender.subscribe();

            let app = create_router(state.clone(), RouterConfig::default());

            let body = PushLogsRequest {
                step_id: step.id,
                step_name: "build".to_string(),
                stream: LogStream::Stdout,
                lines: vec![
                    "Compiling ironflow v0.1.0".to_string(),
                    "Finished in 2.3s".to_string(),
                ],
            };

            let req = Request::builder()
                .method("POST")
                .uri(format!("/api/v1/internal/runs/{}/logs", run.id))
                .header("authorization", "Bearer test-worker-token")
                .header("content-type", "application/json")
                .body(Body::from(to_string(&body).unwrap()))
                .unwrap();

            let resp = app.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);

            let resp_body = resp.into_body().collect().await.unwrap().to_bytes();
            let json_val: JsonValue = from_slice(&resp_body).unwrap();
            assert_eq!(json_val["data"]["accepted"], 2);

            let event1 = rx.recv().await.unwrap();
            assert_eq!(event1.event_type(), "log_line");

            let event2 = rx.recv().await.unwrap();
            assert_eq!(event2.event_type(), "log_line");
        })
        .await
        .expect("test timed out");
    }

    #[tokio::test]
    async fn push_logs_empty_lines() {
        let state = test_state();
        let run = state
            .store
            .create_run(NewRun {
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

        let app = create_router(state.clone(), RouterConfig::default());

        let body = PushLogsRequest {
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            stream: LogStream::Stdout,
            lines: vec![],
        };

        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/internal/runs/{}/logs", run.id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp_body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&resp_body).unwrap();
        assert_eq!(json_val["data"]["accepted"], 0);
    }

    #[tokio::test]
    async fn push_logs_rejects_unauthenticated() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let run_id = Uuid::now_v7();
        let body = PushLogsRequest {
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            stream: LogStream::Stdout,
            lines: vec!["hello".to_string()],
        };

        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/internal/runs/{}/logs", run_id))
            .header("content-type", "application/json")
            .body(Body::from(to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn push_logs_rejects_too_many_lines() {
        let state = test_state();
        let run = state
            .store
            .create_run(NewRun {
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

        let app = create_router(state.clone(), RouterConfig::default());

        let body = PushLogsRequest {
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            stream: LogStream::Stdout,
            lines: (0..1001).map(|i| format!("line {i}")).collect(),
        };

        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/internal/runs/{}/logs", run.id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn push_logs_rejects_unknown_run() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let body = PushLogsRequest {
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            stream: LogStream::Stdout,
            lines: vec!["hello".to_string()],
        };

        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/internal/runs/{}/logs", Uuid::now_v7()))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
