//! `GET /api/v1/internal/runs/next` — Pick the next pending run for execution.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::Deserialize;

use ironflow_engine::notify::{Event, RunStatusChangedEvent};
use ironflow_store::models::RunStatus;

use crate::entities::lease::validate_lease_ttl;
use crate::entities::parse_worker_capabilities;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Query parameters for [`pick_next_run`].
///
/// Every field is optional: a worker that does not send `worker_id` picks runs
/// without a lease, which keeps workers from an older release working during a
/// rolling upgrade. Those runs are never recovered by the reaper. Likewise, a
/// worker that sends neither `workflows` nor `tags` takes every run.
#[derive(Debug, Deserialize)]
pub struct PickNextQuery {
    /// Identifier of the worker requesting a run.
    #[serde(default)]
    pub worker_id: Option<String>,
    /// Lease duration in seconds. Defaults to 90 when `worker_id` is set.
    #[serde(default)]
    pub lease_ttl_secs: Option<u64>,
    /// Comma-separated workflow names the worker registered. Absent means
    /// any workflow.
    #[serde(default)]
    pub workflows: Option<String>,
    /// Comma-separated tags the worker carries. An empty value means none:
    /// the worker only takes runs that require no tag.
    #[serde(default)]
    pub tags: Option<String>,
}

/// Atomically pick the next pending run the worker can take and transition it
/// to Running.
///
/// A run is eligible when the worker registered its workflow and carries every
/// tag it requires (see [`parse_worker_capabilities`]). A run the worker cannot
/// take stays pending for another worker and does not hold back the runs
/// behind it. The worker is recorded in the
/// [`WorkerRegistry`](crate::worker_registry::WorkerRegistry) so the run detail
/// can tell whether a queued run has a worker able to take it.
///
/// Returns the raw store [`Run`] entity or null if no pending runs are available.
/// Internal routes return store entities (not public DTOs) because the worker
/// needs the full `FsmState<RunStatus>` and `payload` fields.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if `worker_id` is blank or `lease_ttl_secs`
/// is outside `1..=3600`, or if a tag is invalid.
pub async fn pick_next_run(
    State(state): State<AppState>,
    Query(query): Query<PickNextQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let lease = validate_lease_ttl(query.worker_id, query.lease_ttl_secs)?;
    let capabilities =
        parse_worker_capabilities(query.workflows.as_deref(), query.tags.as_deref())?;
    if let Some(ref lease) = lease {
        state
            .worker_registry
            .record(&lease.worker_id, capabilities.clone());
    }
    let run = state
        .store
        .pick_next_pending_for(lease, capabilities)
        .await?;

    if let Some(ref picked) = run {
        state
            .engine
            .event_publisher()
            .publish(Event::RunStatusChanged(RunStatusChangedEvent {
                run_id: picked.id,
                workflow_name: picked.workflow_name.clone(),
                from: RunStatus::Pending,
                to: RunStatus::Running,
                error: None,
                cost_usd: Decimal::ZERO,
                duration_ms: 0,
                labels: picked.labels.clone(),
                at: Utc::now(),
            }));
    }

    Ok(ok(run))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use serde_json::{Value as JsonValue, from_slice, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

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
    async fn pick_next_returns_pending_run() {
        let state = test_state();
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
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run();

        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["id"], run.id.to_string());
    }

    #[tokio::test]
    async fn pick_next_empty_returns_null() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert!(json_val["data"].is_null());
    }

    #[tokio::test]
    async fn pick_next_with_worker_id_attaches_lease() {
        let state = test_state();
        state
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
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap();

        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next?worker_id=worker-1&lease_ttl_secs=90")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["worker_id"], "worker-1");
        assert!(json_val["data"]["lease_expires_at"].is_string());
    }

    #[tokio::test]
    async fn pick_next_without_worker_id_leaves_run_unowned() {
        let state = test_state();
        state
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
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap();

        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert!(json_val["data"]["worker_id"].is_null());
        assert!(json_val["data"]["lease_expires_at"].is_null());
    }

    #[tokio::test]
    async fn pick_next_rejects_out_of_range_lease_ttl() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next?worker_id=worker-1&lease_ttl_secs=0")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn pick_next_rejects_blank_worker_id() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next?worker_id=%20%20")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn pick_next_transitions_to_running() {
        let state = test_state();
        let _run = state
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
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run();

        let app = create_router(state.clone(), RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["status"]["state"], "running");
    }

    async fn create_tagged(state: &AppState, workflow: &str, tags: &[&str]) -> Uuid {
        state
            .store
            .create_run(NewRun {
                workflow_name: workflow.to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: tags.iter().map(|t| (*t).to_string()).collect(),
            })
            .await
            .unwrap()
            .into_run()
            .id
    }

    async fn pick(state: &AppState, query: &str) -> (StatusCode, JsonValue) {
        let app = create_router(state.clone(), RouterConfig::default());
        let req = Request::builder()
            .uri(format!("/api/v1/internal/runs/next?{query}"))
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        (status, from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn pick_next_skips_run_whose_tags_the_worker_lacks() {
        let state = test_state();
        let gpu = create_tagged(&state, "transcode", &["gpu"]).await;
        let plain = create_tagged(&state, "transcode", &[]).await;

        let (status, body) = pick(&state, "worker_id=cpu-1&tags=arm").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["id"], plain.to_string());

        let (_, body) = pick(&state, "worker_id=cpu-1&tags=arm").await;
        assert!(body["data"].is_null());

        let (_, body) = pick(&state, "worker_id=gpu-1&tags=gpu,arm").await;
        assert_eq!(body["data"]["id"], gpu.to_string());
        assert_eq!(body["data"]["worker_tags"], json!(["gpu"]));
    }

    #[tokio::test]
    async fn pick_next_skips_workflow_the_worker_did_not_register() {
        let state = test_state();
        create_tagged(&state, "build", &[]).await;
        let deploy = create_tagged(&state, "deploy", &[]).await;

        let (status, body) = pick(&state, "worker_id=w-1&workflows=deploy,lint&tags=").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["id"], deploy.to_string());

        let (_, body) = pick(&state, "worker_id=w-1&workflows=deploy,lint&tags=").await;
        assert!(body["data"].is_null());
    }

    #[tokio::test]
    async fn pick_next_without_capabilities_takes_tagged_run() {
        let state = test_state();
        let gpu = create_tagged(&state, "transcode", &["gpu"]).await;

        let (status, body) = pick(&state, "worker_id=legacy").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["id"], gpu.to_string());
    }

    #[tokio::test]
    async fn pick_next_rejects_invalid_tag() {
        let state = test_state();
        create_tagged(&state, "transcode", &[]).await;

        let (status, _) = pick(&state, "worker_id=w-1&tags=two%20words").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (_, body) = pick(&state, "worker_id=w-1").await;
        assert!(
            !body["data"].is_null(),
            "the rejected request must not pick a run"
        );
    }

    #[tokio::test]
    async fn pick_next_records_the_worker_and_its_tags() {
        let state = test_state();
        pick(&state, "worker_id=cpu-1&tags=arm").await;
        pick(&state, "worker_id=gpu-1&tags=gpu").await;
        pick(&state, "tags=gpu").await;

        let routing = state
            .worker_registry
            .routing_for("transcode", &["gpu".to_string()]);
        assert_eq!(
            routing.seen_workers, 2,
            "a request without worker_id is not recorded"
        );
        assert_eq!(routing.eligible_workers, 1);
    }
}
