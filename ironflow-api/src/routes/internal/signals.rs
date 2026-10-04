//! Internal signal routes: the worker side of `ctx.wait_for_signal`.
//!
//! - `GET /api/v1/internal/signals?name=&key=&since=` lists the signals a
//!   newly opened wait step may consume.
//! - `POST /api/v1/internal/steps/:id/signal-resolution` resolves a waiting
//!   signal step.
//! - `POST /api/v1/internal/runs/:id/signal-suspension` puts a run to sleep on
//!   its waiting signal step.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use ironflow_store::error::StoreError;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Query of `GET /internal/signals`.
#[derive(Debug, Deserialize)]
pub struct SignalsForKeyQuery {
    /// Signal name.
    pub name: String,
    /// Occurrence key.
    pub key: String,
    /// Only signals received at or after this instant.
    pub since: DateTime<Utc>,
}

/// Body of `POST /internal/steps/:id/signal-resolution`.
#[derive(Debug, Deserialize)]
pub struct SignalResolutionBody {
    /// Output to record on the step.
    pub output: Value,
}

/// Body of `POST /internal/runs/:id/signal-suspension`.
#[derive(Debug, Deserialize)]
pub struct SignalSuspensionBody {
    /// The waiting signal step.
    pub step_id: Uuid,
    /// When the wait times out.
    pub deadline_at: DateTime<Utc>,
}

/// Map the store errors of the signal routes to API errors.
fn signal_store_error(err: StoreError) -> ApiError {
    match err {
        StoreError::RunNotFound(id) => ApiError::RunNotFound(id),
        StoreError::StepNotFound(id) => ApiError::StepNotFound(id),
        StoreError::InvalidTransition { from, to } => {
            ApiError::Conflict(format!("invalid run transition: {from:?} -> {to:?}"))
        }
        other => ApiError::Store(other),
    }
}

/// List the signals for `(name, key)` received since `since`, oldest first.
///
/// # Errors
///
/// Returns 500 on store failure.
pub async fn list_signals_for_key(
    State(state): State<AppState>,
    Query(query): Query<SignalsForKeyQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let signals = state
        .store
        .list_signals_for_key(&query.name, &query.key, query.since)
        .await?;
    Ok(ok(signals))
}

/// Resolve a waiting signal step with the given output.
///
/// # Errors
///
/// Returns 404 if the step does not exist, 500 on store failure.
pub async fn resolve_signal_step(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SignalResolutionBody>,
) -> Result<impl IntoResponse, ApiError> {
    let resolution = state
        .store
        .resolve_signal_step(id, body.output)
        .await
        .map_err(signal_store_error)?;
    Ok(ok(resolution))
}

/// Put a running run to sleep on its waiting signal step.
///
/// Returns `true` when the step is still waiting, `false` when a signal
/// resolved it in the meantime (the run is then due right away).
///
/// # Errors
///
/// Returns 404 if the run or the step does not exist, 409 if the run is not
/// `Running`, 500 on store failure.
pub async fn suspend_run_on_signal(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SignalSuspensionBody>,
) -> Result<impl IntoResponse, ApiError> {
    let waiting = state
        .store
        .suspend_run_on_signal(id, body.step_id, body.deadline_at)
        .await
        .map_err(signal_store_error)?;
    Ok(ok(waiting))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::TimeDelta;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{
        NewRun, NewSignal, NewStep, RunStatus, StepKind, StepStatus, StepUpdate, TriggerKind,
        step_trace_id,
    };
    use serde_json::{from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use crate::routes::{RouterConfig, create_router};

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

    /// A `Running` run holding a `Running` signal step on `("demo.done", "k1")`.
    async fn waiting_step(state: &AppState) -> (Uuid, Uuid) {
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
                created_by: None,
                idempotency_key: None,
                concurrency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();
        state
            .store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        let step = state
            .store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "wait", 0),
                name: "wait".to_string(),
                kind: StepKind::Signal,
                position: 0,
                input: Some(json!({"name": "demo.done", "key": "k1", "schema": {}})),
                is_error_handler: false,
            })
            .await
            .unwrap();
        state
            .store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();
        (run.id, step.id)
    }

    async fn call(state: AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let app = create_router(state, RouterConfig::default());
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        // Extractor rejections (e.g. a malformed query) are plain text, not JSON.
        let value = from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn internal_signals_for_key_lists_received_signals() {
        let state = test_state();
        let since = Utc::now() - TimeDelta::seconds(5);
        state
            .store
            .insert_signal(NewSignal {
                name: "demo.done".to_string(),
                key: "k1".to_string(),
                payload: json!({"ok": true}),
                idempotency_id: None,
            })
            .await
            .unwrap();

        let uri = format!(
            "/api/v1/internal/signals?name=demo.done&key=k1&since={}",
            since.timestamp_millis()
        );
        let (status, _) = call(state.clone(), "GET", &uri, Value::Null).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "since must be RFC 3339");

        let since = since.to_rfc3339().replace('+', "%2B");
        let uri = format!("/api/v1/internal/signals?name=demo.done&key=k1&since={since}");
        let (status, body) = call(state, "GET", &uri, Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
        assert_eq!(body["data"][0]["payload"], json!({"ok": true}));
    }

    #[tokio::test]
    async fn internal_signal_resolution_resolves_the_step_once() {
        let state = test_state();
        let (run_id, step_id) = waiting_step(&state).await;
        let uri = format!("/api/v1/internal/steps/{step_id}/signal-resolution");

        let (status, body) = call(
            state.clone(),
            "POST",
            &uri,
            json!({"output": {"timed_out": true}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["outcome"], "resolved");
        assert_eq!(body["data"]["run_id"], json!(run_id));
        assert_eq!(body["data"]["run_resumed"], false);

        let (status, body) = call(state, "POST", &uri, json!({"output": {}})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["outcome"], "not_waiting");
        assert_eq!(body["data"]["output"], json!({"timed_out": true}));
    }

    #[tokio::test]
    async fn internal_signal_resolution_unknown_step_returns_404() {
        let state = test_state();
        let uri = format!(
            "/api/v1/internal/steps/{}/signal-resolution",
            Uuid::now_v7()
        );
        let (status, _) = call(state, "POST", &uri, json!({"output": {}})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn internal_signal_suspension_puts_the_run_to_sleep() {
        let state = test_state();
        let (run_id, step_id) = waiting_step(&state).await;
        let deadline = Utc::now() + TimeDelta::hours(1);
        let uri = format!("/api/v1/internal/runs/{run_id}/signal-suspension");

        let (status, body) = call(
            state.clone(),
            "POST",
            &uri,
            json!({"step_id": step_id, "deadline_at": deadline}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"], true);

        let run = state.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Sleeping);
        assert_eq!(run.scheduled_at, Some(deadline));

        // Already sleeping: a second suspension is refused.
        let (status, _) = call(
            state,
            "POST",
            &uri,
            json!({"step_id": step_id, "deadline_at": deadline}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn internal_signal_routes_require_the_worker_token() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/internal/signals?name=a&key=b&since=2026-01-01T00:00:00Z")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
