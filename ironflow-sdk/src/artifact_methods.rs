//! Artifact download methods for [`IronflowClient`].

use uuid::Uuid;

use crate::client::{ArtifactDownload, IronflowClient};
use crate::error::Error;

impl IronflowClient {
    /// Download an artifact by step name.
    ///
    /// Resolves `step_name` to a step ID via [`get_run`](Self::get_run),
    /// then downloads the artifact. The SHA-256 digest is taken from the
    /// step's artifact metadata (no extra round-trip for the hash).
    ///
    /// Use [`download_artifact_by_step_id`](Self::download_artifact_by_step_id)
    /// to skip the run detail lookup when you already have the step ID.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 (run, step or artifact not found) or 401.
    /// Returns [`Error::Deserialize`] if no step matches `step_name`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let run_id = Uuid::nil();
    /// let artifact = client.download_artifact(run_id, "build", "report.html").await?;
    /// let _ = artifact.content_type;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn download_artifact(
        &self,
        run_id: Uuid,
        step_name: &str,
        artifact_name: &str,
    ) -> Result<ArtifactDownload, Error> {
        let run_detail = self.get_run(run_id).await?;
        let step = run_detail
            .data
            .steps
            .iter()
            .find(|s| s.name == step_name)
            .ok_or_else(|| {
                Error::Deserialize(format!("step '{step_name}' not found in run {run_id}"))
            })?;

        let sha256 = step
            .artifacts
            .iter()
            .find(|a| a.name == artifact_name)
            .map(|a| a.sha256.clone())
            .unwrap_or_default();

        let mut download = self.download_raw(run_id, step.id, artifact_name).await?;
        if !sha256.is_empty() {
            download.sha256 = sha256;
        }
        Ok(download)
    }

    /// Download an artifact by step ID (no extra round-trip).
    ///
    /// Calls `GET /api/v1/runs/{run_id}/steps/{step_id}/artifacts/{name}`
    /// and returns the raw bytes with content type.
    ///
    /// The SHA-256 field is empty because the download route does not
    /// include it. Use [`download_artifact`](Self::download_artifact)
    /// (by step name) to get the digest from the step metadata.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] on 404 (run, step or artifact not found),
    /// 401 or 501 (artifact storage not configured).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let run_id = Uuid::nil();
    /// let step_id = Uuid::nil();
    /// let artifact = client
    ///     .download_artifact_by_step_id(run_id, step_id, "report.html")
    ///     .await?;
    /// let _ = artifact.content_type;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn download_artifact_by_step_id(
        &self,
        run_id: Uuid,
        step_id: Uuid,
        artifact_name: &str,
    ) -> Result<ArtifactDownload, Error> {
        self.download_raw(run_id, step_id, artifact_name).await
    }

    /// Internal: fetch the raw artifact bytes.
    async fn download_raw(
        &self,
        run_id: Uuid,
        step_id: Uuid,
        artifact_name: &str,
    ) -> Result<ArtifactDownload, Error> {
        let path = format!("/api/v1/runs/{run_id}/steps/{step_id}/artifacts/{artifact_name}");
        let response = self.send_with_retry(self.get(&path)).await?;

        if !response.status().is_success() {
            return Err(Self::into_api_error(response).await);
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let bytes = response.bytes().await?;

        Ok(ArtifactDownload {
            bytes,
            content_type,
            sha256: String::new(),
        })
    }
}
