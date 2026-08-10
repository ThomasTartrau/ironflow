//! `GET /api/v1/internal/runs/:id/artifacts` — every artifact of a run.
//!
//! The worker resolves declared inputs on its side: it fetches the run's steps
//! and its artifacts, then matches them locally. That keeps the resolution rule
//! in one place -- the store trait -- instead of duplicating it in a route.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// List every artifact recorded for a run, across steps and attempts.
///
/// Returns raw store `Artifact` entities — internal routes skip the public DTO
/// so the worker can deserialize the full entity, storage key included.
pub async fn list_artifacts(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let artifacts = state.store.list_artifacts_for_run(run_id).await?;
    Ok(ok(artifacts))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_store::models::{NewArtifact, NewStep, StepKind};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::routes::test_helpers::{create_run, test_state};
    use crate::routes::{RouterConfig, create_router};

    fn request(run_id: Uuid, token: Option<&str>) -> Request<Body> {
        let mut builder =
            Request::builder().uri(format!("/api/v1/internal/runs/{run_id}/artifacts"));
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        builder.body(Body::empty()).expect("request")
    }

    #[tokio::test]
    async fn returns_the_artifacts_of_the_run() {
        let state = test_state();
        let run = create_run(&state).await;
        let step = state
            .store
            .create_step(NewStep {
                run_id: run.id,
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("create step");
        let id = Uuid::now_v7();
        state
            .store
            .create_artifact(NewArtifact {
                id,
                run_id: run.id,
                step_id: step.id,
                name: "report.html".to_string(),
                storage_key: format!("artifacts/{}/{}/{id}", run.id, step.id),
                content_type: "text/html".to_string(),
                size_bytes: 7,
                sha256: "0".repeat(64),
            })
            .await
            .expect("record artifact");

        let app = create_router(state, RouterConfig::default());
        let resp = app
            .oneshot(request(run.id, Some("test-worker-token")))
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let json: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["data"][0]["name"], "report.html");
        // The worker needs the storage key to be present on the raw entity.
        assert!(json["data"][0]["storage_key"].is_string());
    }

    #[tokio::test]
    async fn a_run_without_artifacts_returns_an_empty_list() {
        let state = test_state();
        let run = create_run(&state).await;

        let app = create_router(state, RouterConfig::default());
        let resp = app
            .oneshot(request(run.id, Some("test-worker-token")))
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let json: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["data"].as_array().expect("array").len(), 0);
    }

    #[tokio::test]
    async fn a_request_without_the_worker_token_is_rejected() {
        let state = test_state();
        let run = create_run(&state).await;

        let app = create_router(state, RouterConfig::default());
        let resp = app.oneshot(request(run.id, None)).await.expect("response");

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
