//! `GET /api/v1/runs/:id/steps/:step_id/artifacts/:name` — download an artifact.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use futures_util::TryStreamExt;
use ironflow_auth::extractor::Authenticated;
use ironflow_store::models::Artifact;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

/// Download one artifact produced by a step.
///
/// Streams the stored bytes with the MIME type recorded at upload time. The
/// artifact must belong to both the run and the step named in the path, so an
/// id guessed from another run resolves to a 404 rather than a leak.
///
/// Requires the same permission as reading the run itself.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/runs/{id}/steps/{step_id}/artifacts/{name}",
        tags = ["runs"],
        params(
            ("id" = Uuid, Path, description = "Run ID"),
            ("step_id" = Uuid, Path, description = "Step ID"),
            ("name" = String, Path, description = "Artifact name"),
        ),
        responses(
            (status = 200, description = "Artifact content", content_type = "application/octet-stream"),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Run, step or artifact not found"),
            (status = 501, description = "Artifact storage is not configured")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn download_artifact(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path((run_id, step_id, name)): Path<(Uuid, Uuid, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let blob_store = state.blob_store_or_501()?;

    let artifact = state
        .store
        .get_artifact(step_id, &name)
        .await?
        .filter(|artifact| artifact.run_id == run_id)
        .ok_or_else(|| ApiError::ArtifactNotFound(name.clone()))?;

    let stream = blob_store
        .get(&artifact.storage_key)
        .await
        .map_err(|err| match err {
            // The metadata row outlived its blob. Nothing the caller can act on
            // beyond "it is gone", so report it as a plain 404.
            ironflow_artifacts::error::ArtifactError::NotFound(_) => {
                ApiError::ArtifactNotFound(name.clone())
            }
            other => ApiError::Internal(other.to_string()),
        })?;

    let body = Body::from_stream(stream.map_err(std::io::Error::other));

    Ok(artifact_response(&artifact, body))
}

/// Build the download response: recorded MIME type, size, and a filename.
fn artifact_response(artifact: &Artifact, body: Body) -> Response {
    (
        [
            (header::CONTENT_TYPE, artifact.content_type.clone()),
            (header::CONTENT_LENGTH, artifact.size_bytes.to_string()),
            (
                header::CONTENT_DISPOSITION,
                // The name passed a strict whitelist at upload time, so it holds
                // no quote or control character to escape here.
                format!("attachment; filename=\"{}\"", artifact.name),
            ),
        ],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use ironflow_artifacts::blob_store::BlobStore;
    use ironflow_artifacts::local::LocalBlobStore;
    use ironflow_artifacts::stream_from_bytes;
    use ironflow_store::models::{NewArtifact, NewStep, StepKind};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;
    use crate::routes::test_helpers::{auth_header, create_run, test_state};

    /// A state with artifact storage, holding one stored artifact.
    struct Fixture {
        state: AppState,
        run_id: Uuid,
        step_id: Uuid,
        _dir: TempDir,
    }

    async fn fixture() -> Fixture {
        let dir = TempDir::new().expect("temp dir");
        let blob: Arc<dyn BlobStore> = Arc::new(LocalBlobStore::new(dir.path()));
        let state = test_state().with_blob_store(blob.clone());

        let run = create_run(&state).await;
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
            .expect("create step");

        let id = Uuid::now_v7();
        let key = format!("artifacts/{}/{}/{id}", run.id, step.id);
        let digest = blob
            .put(&key, stream_from_bytes(b"<html/>".to_vec()))
            .await
            .expect("put blob");

        state
            .store
            .create_artifact(NewArtifact {
                id,
                run_id: run.id,
                step_id: step.id,
                name: "report.html".to_string(),
                storage_key: key,
                content_type: "text/html".to_string(),
                size_bytes: digest.size_bytes,
                sha256: digest.sha256,
            })
            .await
            .expect("record artifact");

        Fixture {
            state,
            run_id: run.id,
            step_id: step.id,
            _dir: dir,
        }
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .route(
                "/runs/{id}/steps/{step_id}/artifacts/{name}",
                get(download_artifact),
            )
            .with_state(state)
    }

    fn request(state: &AppState, uri: String) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", auth_header(state))
            .body(Body::empty())
            .expect("request")
    }

    #[tokio::test]
    async fn serves_the_content_with_its_recorded_type() {
        let fixture = fixture().await;
        let uri = format!(
            "/runs/{}/steps/{}/artifacts/report.html",
            fixture.run_id, fixture.step_id
        );
        let req = request(&fixture.state, uri);

        let resp = app(fixture.state).oneshot(req).await.expect("response");

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "text/html");
        assert_eq!(resp.headers()[header::CONTENT_LENGTH], "7");
        assert_eq!(
            resp.headers()[header::CONTENT_DISPOSITION],
            "attachment; filename=\"report.html\""
        );

        let body = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
        assert_eq!(&body[..], b"<html/>");
    }

    #[tokio::test]
    async fn an_unknown_name_is_not_found() {
        let fixture = fixture().await;
        let uri = format!(
            "/runs/{}/steps/{}/artifacts/nope.html",
            fixture.run_id, fixture.step_id
        );
        let req = request(&fixture.state, uri);

        let resp = app(fixture.state).oneshot(req).await.expect("response");

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_artifact_of_another_run_is_not_found() {
        let fixture = fixture().await;
        let other_run = create_run(&fixture.state).await;
        let uri = format!(
            "/runs/{}/steps/{}/artifacts/report.html",
            other_run.id, fixture.step_id
        );
        let req = request(&fixture.state, uri);

        let resp = app(fixture.state).oneshot(req).await.expect("response");

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_artifact_of_another_step_is_not_found() {
        let fixture = fixture().await;
        let uri = format!(
            "/runs/{}/steps/{}/artifacts/report.html",
            fixture.run_id,
            Uuid::now_v7()
        );
        let req = request(&fixture.state, uri);

        let resp = app(fixture.state).oneshot(req).await.expect("response");

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_anonymous_request_is_rejected() {
        let fixture = fixture().await;
        let uri = format!(
            "/runs/{}/steps/{}/artifacts/report.html",
            fixture.run_id, fixture.step_id
        );
        let req = Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request");

        let resp = app(fixture.state).oneshot(req).await.expect("response");

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn without_a_backend_the_route_reports_not_implemented() {
        let state = test_state();
        let run = create_run(&state).await;
        let uri = format!(
            "/runs/{}/steps/{}/artifacts/report.html",
            run.id,
            Uuid::now_v7()
        );
        let req = request(&state, uri);

        let resp = app(state).oneshot(req).await.expect("response");

        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn a_record_whose_blob_vanished_is_not_found() {
        let fixture = fixture().await;
        let artifact = fixture
            .state
            .store
            .get_artifact(fixture.step_id, "report.html")
            .await
            .expect("get")
            .expect("present");
        fixture
            .state
            .blob_store
            .as_ref()
            .expect("blob store")
            .delete(&artifact.storage_key)
            .await
            .expect("delete blob");

        let uri = format!(
            "/runs/{}/steps/{}/artifacts/report.html",
            fixture.run_id, fixture.step_id
        );
        let req = request(&fixture.state, uri);

        let resp = app(fixture.state).oneshot(req).await.expect("response");

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
