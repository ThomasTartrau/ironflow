//! `POST /api/v1/internal/runs/:id/steps/:step_id/artifacts/:name` — store an artifact.
//!
//! A remote worker has no access to the storage backend: it streams the bytes
//! here and the API writes them. That keeps storage credentials on the API side
//! and makes the local and object-storage backends behave identically.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::header;
use axum::response::IntoResponse;
use futures_util::TryStreamExt;
use uuid::Uuid;

use ironflow_artifacts::blob_store::ByteStream;
use ironflow_artifacts::error::ArtifactError;
use ironflow_artifacts::name::{guess_content_type, storage_key, validate_artifact_name};
use ironflow_store::models::NewArtifact;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Store the bytes of an artifact and record its metadata.
///
/// The MIME type comes from the `Content-Type` header, falling back to a guess
/// from the file name. The blob is written first: a crash before the metadata
/// row leaves an unreferenced blob, which is invisible, rather than a record
/// pointing at nothing.
///
/// Returns the raw store `Artifact` entity — internal routes skip the public
/// DTO so the worker can deserialize the full entity.
pub async fn upload_artifact(
    State(state): State<AppState>,
    Path((run_id, step_id, name)): Path<(Uuid, Uuid, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<impl IntoResponse, ApiError> {
    let blob_store = state.blob_store_or_501()?;

    validate_artifact_name(&name).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    // A step of another run must not be able to hang its artifacts on this run.
    let step = state
        .store
        .get_step(step_id)
        .await?
        .filter(|step| step.run_id == run_id)
        .ok_or(ApiError::StepNotFound(step_id))?;

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| guess_content_type(&name));

    let id = Uuid::now_v7();
    let key = storage_key(run_id, step_id, id);

    let content: ByteStream = Box::pin(
        body.into_data_stream()
            .map_err(|err| ArtifactError::Io(err.to_string())),
    );

    let digest = blob_store
        .put(&key, content)
        .await
        .map_err(|err| match err {
            ArtifactError::TooLarge { .. } => ApiError::ArtifactTooLarge,
            other => ApiError::Internal(other.to_string()),
        })?;

    let recorded = state
        .store
        .create_artifact(NewArtifact {
            id,
            run_id: step.run_id,
            step_id,
            name,
            storage_key: key.clone(),
            content_type,
            size_bytes: digest.size_bytes,
            sha256: digest.sha256,
        })
        .await;

    match recorded {
        Ok(artifact) => Ok(ok(artifact)),
        Err(err) => {
            // The blob is unreachable without its record; drop it rather than
            // leave a byte-for-byte orphan behind a known failure.
            if let Err(cleanup) = blob_store.delete(&key).await {
                tracing::warn!(
                    storage_key = %key,
                    error = %cleanup,
                    "failed to remove the blob of an unrecorded artifact"
                );
            }
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_artifacts::blob_store::BlobStore;
    use ironflow_artifacts::local::LocalBlobStore;
    use ironflow_store::models::{NewStep, StepKind};
    use serde_json::Value;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;
    use crate::routes::test_helpers::{create_run, test_state};
    use crate::routes::{RouterConfig, create_router};

    struct Fixture {
        state: AppState,
        run_id: Uuid,
        step_id: Uuid,
        _dir: TempDir,
    }

    /// A state with artifact storage and one persisted step.
    async fn fixture() -> Fixture {
        let dir = TempDir::new().expect("temp dir");
        let blob: Arc<dyn BlobStore> = Arc::new(LocalBlobStore::new(dir.path()).max_bytes(64));
        let state = test_state().with_blob_store(blob);

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

        Fixture {
            state,
            run_id: run.id,
            step_id: step.id,
            _dir: dir,
        }
    }

    fn upload_request(
        run_id: Uuid,
        step_id: Uuid,
        name: &str,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/internal/runs/{run_id}/steps/{step_id}/artifacts/{name}"
            ))
            .header("authorization", "Bearer test-worker-token");

        if let Some(content_type) = content_type {
            builder = builder.header("content-type", content_type);
        }

        builder.body(Body::from(body)).expect("request")
    }

    #[tokio::test]
    async fn stores_the_bytes_and_records_the_metadata() {
        let fixture = fixture().await;
        let app = create_router(fixture.state.clone(), RouterConfig::default());

        let resp = app
            .oneshot(upload_request(
                fixture.run_id,
                fixture.step_id,
                "report.html",
                None,
                b"<html/>".to_vec(),
            ))
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let json: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["data"]["name"], "report.html");
        assert_eq!(json["data"]["size_bytes"], 7);
        assert_eq!(json["data"]["content_type"], "text/html");
    }

    #[tokio::test]
    async fn an_explicit_content_type_header_wins() {
        let fixture = fixture().await;
        let app = create_router(fixture.state.clone(), RouterConfig::default());

        let resp = app
            .oneshot(upload_request(
                fixture.run_id,
                fixture.step_id,
                "data.txt",
                Some("application/json"),
                b"{}".to_vec(),
            ))
            .await
            .expect("response");

        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let json: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["data"]["content_type"], "application/json");
    }

    #[tokio::test]
    async fn a_name_outside_the_whitelist_is_rejected() {
        let fixture = fixture().await;
        let app = create_router(fixture.state.clone(), RouterConfig::default());

        let resp = app
            .oneshot(upload_request(
                fixture.run_id,
                fixture.step_id,
                "-rf",
                None,
                b"x".to_vec(),
            ))
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_step_of_another_run_is_refused() {
        let fixture = fixture().await;
        let other_run = create_run(&fixture.state).await;
        let app = create_router(fixture.state.clone(), RouterConfig::default());

        let resp = app
            .oneshot(upload_request(
                other_run.id,
                fixture.step_id,
                "report.html",
                None,
                b"x".to_vec(),
            ))
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_same_name_twice_conflicts() {
        let fixture = fixture().await;
        let app = create_router(fixture.state.clone(), RouterConfig::default());

        app.clone()
            .oneshot(upload_request(
                fixture.run_id,
                fixture.step_id,
                "report.html",
                None,
                b"x".to_vec(),
            ))
            .await
            .expect("first");

        let resp = app
            .oneshot(upload_request(
                fixture.run_id,
                fixture.step_id,
                "report.html",
                None,
                b"y".to_vec(),
            ))
            .await
            .expect("second");

        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn a_payload_over_the_limit_is_refused() {
        let fixture = fixture().await;
        let app = create_router(fixture.state.clone(), RouterConfig::default());

        let resp = app
            .oneshot(upload_request(
                fixture.run_id,
                fixture.step_id,
                "big.bin",
                None,
                vec![0u8; 128],
            ))
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn a_refused_upload_records_nothing() {
        let fixture = fixture().await;
        let app = create_router(fixture.state.clone(), RouterConfig::default());

        app.oneshot(upload_request(
            fixture.run_id,
            fixture.step_id,
            "big.bin",
            None,
            vec![0u8; 128],
        ))
        .await
        .expect("response");

        assert!(
            fixture
                .state
                .store
                .list_artifacts_for_run(fixture.run_id)
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_request_without_the_worker_token_is_rejected() {
        let fixture = fixture().await;
        let app = create_router(fixture.state.clone(), RouterConfig::default());

        let req = Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/internal/runs/{}/steps/{}/artifacts/report.html",
                fixture.run_id, fixture.step_id
            ))
            .body(Body::from("x"))
            .expect("request");

        let resp = app.oneshot(req).await.expect("response");

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn without_a_backend_the_route_reports_not_implemented() {
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
        let app = create_router(state, RouterConfig::default());

        let resp = app
            .oneshot(upload_request(
                run.id,
                step.id,
                "report.html",
                None,
                b"x".to_vec(),
            ))
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    }
}
