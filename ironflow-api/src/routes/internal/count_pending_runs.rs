//! `GET /api/v1/internal/runs/pending-count` — Number of runs waiting for a worker.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use ironflow_store::entities::{ConcurrencyGroupBacklog, RunFilter, RunStatus};

use crate::entities::parse_worker_capabilities;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Query parameters for [`count_pending_runs`].
///
/// The same `workflows` and `tags` a worker sends to the pick route. Absent,
/// every pending run is counted.
#[derive(Debug, Deserialize)]
pub struct PendingCountQuery {
    /// Comma-separated workflow names the worker registered. Absent means
    /// any workflow.
    #[serde(default)]
    pub workflows: Option<String>,
    /// Comma-separated tags the worker carries. An empty value means none.
    #[serde(default)]
    pub tags: Option<String>,
}

/// Queue depth as seen by the store.
#[derive(Serialize)]
struct PendingCount {
    pending_runs: u64,
    blocked_by_group: Vec<ConcurrencyGroupBacklog>,
}

/// Count the runs in the `Pending` state, and the due runs held back by each
/// saturated concurrency group.
///
/// A worker has no store of its own: it reads these counts to publish
/// `ironflow_worker_queue_depth` and `ironflow_worker_queue_blocked_runs`.
///
/// When the worker sends its capabilities, `pending_runs` only counts the runs
/// it can take, so a worker does not scale on a queue it cannot drain.
/// `blocked_by_group` stays the count for every worker: a saturated group
/// holds back runs whoever would take them.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if a tag is invalid.
pub async fn count_pending_runs(
    State(state): State<AppState>,
    Query(query): Query<PendingCountQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let eligible_for =
        parse_worker_capabilities(query.workflows.as_deref(), query.tags.as_deref())?;
    let stats = state
        .store
        .get_stats(RunFilter {
            status: Some(RunStatus::Pending),
            eligible_for,
            ..RunFilter::default()
        })
        .await?;
    let blocked_by_group = state.store.count_blocked_runs_by_group().await?;

    Ok(ok(PendingCount {
        pending_runs: stats.total_runs,
        blocked_by_group,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{ConcurrencyLimit, NewRun, RunStatus, TriggerKind};
    use serde_json::{Value as JsonValue, from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

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

    fn new_run() -> NewRun {
        NewRun {
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
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        }
    }

    fn request(authorization: Option<&str>) -> Request<Body> {
        let builder = Request::builder().uri("/api/v1/internal/runs/pending-count");
        let builder = match authorization {
            Some(value) => builder.header("authorization", value),
            None => builder,
        };
        builder.body(Body::empty()).unwrap()
    }

    async fn pending_runs_for(state: &AppState, query: &str) -> (StatusCode, JsonValue) {
        let app = create_router(state.clone(), RouterConfig::default());
        let req = Request::builder()
            .uri(format!("/api/v1/internal/runs/pending-count?{query}"))
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        (status, from_slice(&body).unwrap())
    }

    fn tagged_run(workflow: &str, tags: &[&str]) -> NewRun {
        NewRun {
            workflow_name: workflow.to_string(),
            worker_tags: tags.iter().map(|t| (*t).to_string()).collect(),
            ..new_run()
        }
    }

    #[tokio::test]
    async fn count_pending_runs_with_tags_counts_only_runs_the_worker_can_take() {
        let state = test_state();
        state
            .store
            .create_run(tagged_run("transcode", &["gpu"]))
            .await
            .unwrap();
        state
            .store
            .create_run(tagged_run("transcode", &[]))
            .await
            .unwrap();
        state
            .store
            .create_run(tagged_run("build", &[]))
            .await
            .unwrap();

        let (status, json) = pending_runs_for(&state, "workflows=transcode&tags=").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["pending_runs"], 1);

        let (_, json) = pending_runs_for(&state, "workflows=transcode&tags=gpu").await;
        assert_eq!(json["data"]["pending_runs"], 2);

        let (_, json) = pending_runs_for(&state, "tags=gpu").await;
        assert_eq!(json["data"]["pending_runs"], 3);
    }

    #[tokio::test]
    async fn count_pending_runs_without_tags_counts_every_pending_run() {
        let state = test_state();
        state
            .store
            .create_run(tagged_run("transcode", &["gpu"]))
            .await
            .unwrap();
        state
            .store
            .create_run(tagged_run("build", &[]))
            .await
            .unwrap();

        let (status, json) = pending_runs_for(&state, "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["pending_runs"], 2);
    }

    #[tokio::test]
    async fn count_pending_runs_rejects_invalid_tag() {
        let (status, json) = pending_runs_for(&test_state(), "tags=bad%2Ftag%20x").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "BAD_REQUEST");
        let message = json["error"]["message"].as_str().unwrap();
        assert!(message.contains("bad/tag x"), "{message}");
    }

    #[tokio::test]
    async fn count_pending_runs_counts_only_pending_runs() {
        let state = test_state();
        for _ in 0..2 {
            state.store.create_run(new_run()).await.unwrap();
        }
        let running = state.store.create_run(new_run()).await.unwrap().into_run();
        state
            .store
            .update_run_status(running.id, RunStatus::Running)
            .await
            .unwrap();

        let app = create_router(state, RouterConfig::default());
        let resp = app
            .oneshot(request(Some("Bearer test-worker-token")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json["data"]["pending_runs"], 2);
    }

    #[tokio::test]
    async fn count_pending_runs_is_zero_on_empty_store() {
        let app = create_router(test_state(), RouterConfig::default());
        let resp = app
            .oneshot(request(Some("Bearer test-worker-token")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json["data"]["pending_runs"], 0);
        assert_eq!(json["data"]["blocked_by_group"], json!([]));
    }

    #[tokio::test]
    async fn count_pending_runs_reports_runs_blocked_by_group() {
        let state = test_state();
        let in_group = || NewRun {
            concurrency_limits: vec![ConcurrencyLimit::new("repo:acme", 1)],
            ..new_run()
        };
        let holder = state.store.create_run(in_group()).await.unwrap().into_run();
        state
            .store
            .update_run_status(holder.id, RunStatus::Running)
            .await
            .unwrap();
        for _ in 0..2 {
            state.store.create_run(in_group()).await.unwrap();
        }
        state.store.create_run(new_run()).await.unwrap();

        let app = create_router(state, RouterConfig::default());
        let resp = app
            .oneshot(request(Some("Bearer test-worker-token")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json["data"]["pending_runs"], 3);
        assert_eq!(
            json["data"]["blocked_by_group"],
            json!([{ "group": "repo:acme", "blocked_runs": 2 }])
        );
    }

    #[tokio::test]
    async fn count_pending_runs_rejects_missing_or_wrong_worker_token() {
        for authorization in [None, Some("Bearer wrong-token")] {
            let app = create_router(test_state(), RouterConfig::default());
            let resp = app.oneshot(request(authorization)).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{authorization:?}");

            let body = resp.into_body().collect().await.unwrap().to_bytes();
            let json: JsonValue = from_slice(&body).unwrap();
            assert_eq!(json["error"]["code"], "INVALID_WORKER_TOKEN");
        }
    }
}
