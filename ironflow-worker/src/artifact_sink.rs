//! HTTP-backed [`ArtifactSink`] for the remote worker.
//!
//! A worker has no access to the storage backend: it streams artifact bytes to
//! the API, which writes them and records the metadata. Storage credentials
//! therefore stay on the API side, and the local and object-storage backends
//! behave identically from the worker's point of view.

use bytes::Bytes;
use futures_util::TryStreamExt;
use reqwest::{Body, Client, StatusCode};

use ironflow_artifacts::blob_store::ByteStream;
use ironflow_artifacts::error::ArtifactError;
use ironflow_engine::artifact::{ArtifactFuture, ArtifactSink, ArtifactUpload};
use ironflow_engine::error::EngineError;
use ironflow_store::entities::Artifact;
use ironflow_store::error::StoreError;

/// API response envelope.
#[derive(serde::Deserialize)]
struct ApiResponse<T> {
    data: T,
}

/// [`ArtifactSink`] that uploads and downloads through the internal API routes.
#[derive(Debug, Clone)]
pub struct ApiArtifactSink {
    client: Client,
    base_url: String,
    token: String,
}

impl ApiArtifactSink {
    /// Build a sink against an API base URL and a worker token.
    ///
    /// The client carries no request timeout: an artifact upload is a bulk
    /// transfer whose duration depends on payload size, unlike the short JSON
    /// calls the run store makes.
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    fn artifact_url(&self, run_id: &str, step_id: &str, name: &str) -> String {
        format!(
            "{}/api/v1/internal/runs/{run_id}/steps/{step_id}/artifacts/{name}",
            self.base_url
        )
    }

    fn transport_error(err: reqwest::Error) -> EngineError {
        EngineError::Artifact(ArtifactError::Io(format!("worker HTTP error: {err}")))
    }
}

impl ArtifactSink for ApiArtifactSink {
    fn put<'a>(
        &'a self,
        upload: ArtifactUpload,
        content: ByteStream,
    ) -> ArtifactFuture<'a, Artifact> {
        Box::pin(async move {
            let url = self.artifact_url(
                &upload.run_id.to_string(),
                &upload.step_id.to_string(),
                &upload.name,
            );

            let resp = self
                .client
                .post(url)
                .bearer_auth(&self.token)
                .header(reqwest::header::CONTENT_TYPE, &upload.content_type)
                .body(Body::wrap_stream(content))
                .send()
                .await
                .map_err(Self::transport_error)?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(match status {
                    StatusCode::PAYLOAD_TOO_LARGE => EngineError::Artifact(ArtifactError::Io(
                        format!("artifact {:?} exceeds the API size limit", upload.name),
                    )),
                    StatusCode::CONFLICT => EngineError::Store(StoreError::DuplicateArtifact {
                        step_id: upload.step_id,
                        name: upload.name,
                    }),
                    StatusCode::NOT_IMPLEMENTED => EngineError::ArtifactsUnavailable(
                        "the API server has no artifact storage configured".to_string(),
                    ),
                    _ => EngineError::Artifact(ArtifactError::Io(format!(
                        "artifact upload failed ({status}): {body}"
                    ))),
                });
            }

            let envelope: ApiResponse<Artifact> =
                resp.json().await.map_err(Self::transport_error)?;
            Ok(envelope.data)
        })
    }

    fn get<'a>(&'a self, artifact: &'a Artifact) -> ArtifactFuture<'a, ByteStream> {
        Box::pin(async move {
            let url = self.artifact_url(
                &artifact.run_id.to_string(),
                &artifact.step_id.to_string(),
                &artifact.name,
            );

            let resp = self
                .client
                .get(url)
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(Self::transport_error)?;

            let status = resp.status();
            if !status.is_success() {
                return Err(match status {
                    StatusCode::NOT_FOUND => {
                        EngineError::Artifact(ArtifactError::NotFound(artifact.name.clone()))
                    }
                    StatusCode::NOT_IMPLEMENTED => EngineError::ArtifactsUnavailable(
                        "the API server has no artifact storage configured".to_string(),
                    ),
                    _ => EngineError::Artifact(ArtifactError::Io(format!(
                        "artifact download failed ({status})"
                    ))),
                });
            }

            let stream = resp
                .bytes_stream()
                .map_ok(Bytes::from)
                .map_err(|err| ArtifactError::Io(err.to_string()));

            Ok(Box::pin(stream) as ByteStream)
        })
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn the_url_targets_the_internal_artifact_route() {
        let sink = ApiArtifactSink::new("http://localhost:3000", "token");
        let run = Uuid::nil();
        let step = Uuid::nil();

        let url = sink.artifact_url(&run.to_string(), &step.to_string(), "report.html");

        assert_eq!(
            url,
            format!(
                "http://localhost:3000/api/v1/internal/runs/{run}/steps/{step}/artifacts/report.html"
            )
        );
    }

    #[test]
    fn a_trailing_slash_in_the_base_url_is_dropped() {
        let sink = ApiArtifactSink::new("http://localhost:3000/", "token");

        let url = sink.artifact_url("run", "step", "a.txt");

        assert!(!url.contains("3000//"));
    }
}
