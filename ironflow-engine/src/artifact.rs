//! Artifact production and consumption from a running workflow.
//!
//! [`ArtifactSink`] is the seam between the engine and wherever artifact bytes
//! actually live. The engine never talks to a blob store directly, because the
//! two execution topologies reach storage differently:
//!
//! - in-process (API server, tests): [`DirectArtifactSink`] writes to the blob
//!   store and records the metadata itself;
//! - remote worker: the worker's sink uploads over the internal HTTP API, which
//!   keeps storage credentials on the API side only.
//!
//! Ordering is always "bytes first, metadata second". The metadata row is the
//! source of truth, so a crash in between leaves an unreferenced blob rather
//! than a record pointing at nothing.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::StreamExt;
use glob::glob;
use tokio::fs::{File, create_dir_all};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use uuid::Uuid;

use ironflow_artifacts::blob_store::{BlobStore, ByteStream};
use ironflow_artifacts::error::ArtifactError;
use ironflow_artifacts::name::{guess_content_type, storage_key, validate_artifact_name};
use ironflow_artifacts::stream_from_path;
use ironflow_store::entities::{Artifact, ArtifactLookup, NewArtifact};
use ironflow_store::store::Store;

use crate::config::ShellConfig;
use crate::error::EngineError;

/// Boxed future returned by [`ArtifactSink`] methods -- keeps the trait object safe.
pub type ArtifactFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, EngineError>> + Send + 'a>>;

/// Everything needed to record an artifact, minus the bytes.
///
/// # Examples
///
/// ```
/// use ironflow_engine::artifact::ArtifactUpload;
/// use uuid::Uuid;
///
/// let upload = ArtifactUpload {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     name: "report.html".to_string(),
///     content_type: "text/html".to_string(),
/// };
/// assert_eq!(upload.name, "report.html");
/// ```
#[derive(Debug, Clone)]
pub struct ArtifactUpload {
    /// The run the artifact belongs to.
    pub run_id: Uuid,
    /// The step that produced it.
    pub step_id: Uuid,
    /// User-facing file name, unique within the step.
    pub name: String,
    /// MIME type to serve on download.
    pub content_type: String,
}

/// Where a running workflow reads and writes artifact bytes.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
///
/// use ironflow_artifacts::stream_from_bytes;
/// use ironflow_engine::artifact::{ArtifactSink, ArtifactUpload};
/// use uuid::Uuid;
///
/// # async fn example(sink: Arc<dyn ArtifactSink>, run_id: Uuid, step_id: Uuid)
/// # -> Result<(), ironflow_engine::error::EngineError> {
/// let artifact = sink
///     .put(
///         ArtifactUpload {
///             run_id,
///             step_id,
///             name: "report.json".to_string(),
///             content_type: "application/json".to_string(),
///         },
///         stream_from_bytes(b"{}".to_vec()),
///     )
///     .await?;
///
/// assert_eq!(artifact.size_bytes, 2);
/// # Ok(())
/// # }
/// ```
pub trait ArtifactSink: Send + Sync {
    /// Store the bytes of an artifact and record its metadata.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Artifact`] when the name is invalid or storage
    /// fails, and [`EngineError::Store`] when the metadata cannot be recorded
    /// (for instance when the step already owns that name).
    fn put<'a>(
        &'a self,
        upload: ArtifactUpload,
        content: ByteStream,
    ) -> ArtifactFuture<'a, Artifact>;

    /// Open the bytes of a recorded artifact for reading.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Artifact`] when the blob is missing or storage fails.
    fn get<'a>(&'a self, artifact: &'a Artifact) -> ArtifactFuture<'a, ByteStream>;
}

/// [`ArtifactSink`] backed by a blob store and a run store in the same process.
///
/// Used by the API server and by any in-process engine. A remote worker uses an
/// HTTP-backed sink instead.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
///
/// use ironflow_artifacts::local::LocalBlobStore;
/// use ironflow_engine::artifact::DirectArtifactSink;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
///
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let blob = Arc::new(LocalBlobStore::new("/var/lib/ironflow/artifacts"));
/// let sink = DirectArtifactSink::new(blob, store);
/// # drop(sink);
/// ```
pub struct DirectArtifactSink {
    blob: Arc<dyn BlobStore>,
    store: Arc<dyn Store>,
}

impl DirectArtifactSink {
    /// Build a sink over a blob store and the run store holding the metadata.
    pub fn new(blob: Arc<dyn BlobStore>, store: Arc<dyn Store>) -> Self {
        Self { blob, store }
    }
}

impl std::fmt::Debug for DirectArtifactSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectArtifactSink").finish_non_exhaustive()
    }
}

impl ArtifactSink for DirectArtifactSink {
    fn put<'a>(
        &'a self,
        upload: ArtifactUpload,
        content: ByteStream,
    ) -> ArtifactFuture<'a, Artifact> {
        Box::pin(async move {
            validate_artifact_name(&upload.name)?;

            let id = Uuid::now_v7();
            let key = storage_key(upload.run_id, upload.step_id, id);
            let digest = self.blob.put(&key, content).await?;

            let recorded = self
                .store
                .create_artifact(NewArtifact {
                    id,
                    run_id: upload.run_id,
                    step_id: upload.step_id,
                    name: upload.name,
                    storage_key: key.clone(),
                    content_type: upload.content_type,
                    size_bytes: digest.size_bytes,
                    sha256: digest.sha256,
                })
                .await;

            match recorded {
                Ok(artifact) => Ok(artifact),
                Err(err) => {
                    // The blob is unreachable without its record; drop it rather
                    // than leave a byte-for-byte orphan behind a known failure.
                    if let Err(cleanup) = self.blob.delete(&key).await {
                        warn!(
                            storage_key = %key,
                            error = %cleanup,
                            "failed to remove the blob of an unrecorded artifact"
                        );
                    }
                    Err(err.into())
                }
            }
        })
    }

    fn get<'a>(&'a self, artifact: &'a Artifact) -> ArtifactFuture<'a, ByteStream> {
        Box::pin(async move { Ok(self.blob.get(&artifact.storage_key).await?) })
    }
}

/// Where a step sits in its run, for resolving declared inputs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StepLocation {
    /// Run being executed.
    pub(crate) run_id: Uuid,
    /// Attempt being executed.
    pub(crate) attempt: u32,
    /// Position of the step within the attempt.
    pub(crate) position: u32,
}

/// Write every artifact a step declared as an input into its working directory.
///
/// Runs before the command starts, so the command sees the files it expects.
pub(crate) async fn materialize_inputs(
    sink: &Arc<dyn ArtifactSink>,
    store: &Arc<dyn Store>,
    config: &ShellConfig,
    location: StepLocation,
) -> Result<(), EngineError> {
    let work_dir = working_dir(config);

    for input in &config.inputs {
        let artifact = store
            .find_artifact_for_input(ArtifactLookup {
                run_id: location.run_id,
                attempt: location.attempt,
                before_position: location.position,
                step_name: input.step.clone(),
                name: input.name.clone(),
            })
            .await?
            .ok_or_else(|| EngineError::ArtifactNotFound {
                step: input.step.clone(),
                name: input.name.clone(),
            })?;

        let destination = work_dir.join(input.destination());
        if let Some(parent) = destination.parent() {
            create_dir_all(parent).await.map_err(ArtifactError::from)?;
        }

        let mut content = sink.get(&artifact).await?;
        let mut file = File::create(&destination)
            .await
            .map_err(ArtifactError::from)?;
        while let Some(chunk) = content.next().await {
            file.write_all(&chunk?).await.map_err(ArtifactError::from)?;
        }
        file.flush().await.map_err(ArtifactError::from)?;

        info!(
            run_id = %location.run_id,
            artifact = %input.name,
            produced_by = %input.step,
            destination = %destination.display(),
            "artifact input materialized"
        );
    }

    Ok(())
}

/// Store every file a step declared as an output.
///
/// `step_succeeded` decides how strict the collection is. On success, a pattern
/// that matches nothing fails the step: a declared output that never appeared
/// is a broken contract. On failure the collection is best-effort, because the
/// partial files a failing step leaves behind are usually the useful ones.
pub(crate) async fn collect_outputs(
    sink: &Arc<dyn ArtifactSink>,
    config: &ShellConfig,
    run_id: Uuid,
    step_id: Uuid,
    step_name: &str,
    step_succeeded: bool,
) -> Result<(), EngineError> {
    let work_dir = working_dir(config);

    for output in &config.outputs {
        let pattern = work_dir.join(&output.pattern);
        let pattern = pattern.to_str().ok_or_else(|| {
            EngineError::StepConfig(format!(
                "output pattern {:?} is not valid UTF-8",
                output.pattern
            ))
        })?;

        let matches = glob(pattern)
            .map_err(|err| {
                EngineError::StepConfig(format!(
                    "invalid output pattern {:?}: {err}",
                    output.pattern
                ))
            })?
            .filter_map(Result::ok)
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();

        if matches.is_empty() {
            if step_succeeded {
                return Err(EngineError::MissingArtifact {
                    step: step_name.to_string(),
                    pattern: output.pattern.clone(),
                });
            }
            warn!(
                run_id = %run_id,
                step = %step_name,
                pattern = %output.pattern,
                "declared output matched no file on a failed step"
            );
            continue;
        }

        for path in matches {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    EngineError::StepConfig(format!(
                        "output file {:?} has no valid UTF-8 name",
                        path.display()
                    ))
                })?
                .to_string();

            let content_type = output
                .content_type
                .clone()
                .unwrap_or_else(|| guess_content_type(&name));

            let content = stream_from_path(&path).await?;
            let artifact = sink
                .put(
                    ArtifactUpload {
                        run_id,
                        step_id,
                        name: name.clone(),
                        content_type,
                    },
                    content,
                )
                .await?;

            info!(
                run_id = %run_id,
                step = %step_name,
                artifact = %artifact.name,
                size_bytes = artifact.size_bytes,
                "artifact output stored"
            );
        }
    }

    Ok(())
}

/// Directory a shell step runs in, and the base for its artifact paths.
fn working_dir(config: &ShellConfig) -> PathBuf {
    PathBuf::from(config.dir.as_deref().unwrap_or("."))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use futures_util::TryStreamExt;
    use ironflow_artifacts::local::LocalBlobStore;
    use ironflow_artifacts::stream_from_bytes;
    use ironflow_store::entities::{NewRun, NewStep, StepKind, TriggerKind, step_trace_id};
    use ironflow_store::memory::InMemoryStore;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    async fn sink_with_step() -> (TempDir, DirectArtifactSink, Uuid, Uuid) {
        let dir = TempDir::new().expect("temp dir");
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let blob: Arc<dyn BlobStore> = Arc::new(LocalBlobStore::new(dir.path()));

        let run = store
            .create_run(NewRun {
                workflow_name: "artifacts".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run();

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "build", 0),
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("create step");

        let sink = DirectArtifactSink::new(blob, store);
        (dir, sink, run.id, step.id)
    }

    fn upload(run_id: Uuid, step_id: Uuid, name: &str) -> ArtifactUpload {
        ArtifactUpload {
            run_id,
            step_id,
            name: name.to_string(),
            content_type: "text/plain".to_string(),
        }
    }

    #[tokio::test]
    async fn put_records_size_and_hash() {
        let (_dir, sink, run_id, step_id) = sink_with_step().await;

        let artifact = sink
            .put(
                upload(run_id, step_id, "report.txt"),
                stream_from_bytes(b"abc".to_vec()),
            )
            .await
            .expect("put");

        assert_eq!(artifact.size_bytes, 3);
        assert_eq!(
            artifact.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn put_then_get_roundtrips_the_bytes() {
        let (_dir, sink, run_id, step_id) = sink_with_step().await;

        let artifact = sink
            .put(
                upload(run_id, step_id, "report.txt"),
                stream_from_bytes(b"hello".to_vec()),
            )
            .await
            .expect("put");

        let chunks: Vec<bytes::Bytes> = sink
            .get(&artifact)
            .await
            .expect("get")
            .try_collect()
            .await
            .expect("collect");

        assert_eq!(chunks.concat(), b"hello");
    }

    #[tokio::test]
    async fn the_storage_key_never_embeds_the_name() {
        let (_dir, sink, run_id, step_id) = sink_with_step().await;

        let artifact = sink
            .put(
                upload(run_id, step_id, "report.txt"),
                stream_from_bytes(b"x".to_vec()),
            )
            .await
            .expect("put");

        assert!(!artifact.storage_key.contains("report"));
        assert!(artifact.storage_key.ends_with(&artifact.id.to_string()));
    }

    #[tokio::test]
    async fn an_invalid_name_is_rejected_before_anything_is_written() {
        let (dir, sink, run_id, step_id) = sink_with_step().await;

        let err = sink
            .put(
                upload(run_id, step_id, "../escape"),
                stream_from_bytes(b"x".to_vec()),
            )
            .await
            .expect_err("invalid name");

        assert!(matches!(err, EngineError::Artifact(_)));
        assert!(!dir.path().join("artifacts").exists());
    }

    #[tokio::test]
    async fn a_duplicate_name_fails_and_leaves_no_orphan_blob() {
        let (dir, sink, run_id, step_id) = sink_with_step().await;

        sink.put(
            upload(run_id, step_id, "report.txt"),
            stream_from_bytes(b"first".to_vec()),
        )
        .await
        .expect("first");

        let err = sink
            .put(
                upload(run_id, step_id, "report.txt"),
                stream_from_bytes(b"second".to_vec()),
            )
            .await
            .expect_err("duplicate");

        assert!(matches!(err, EngineError::Store(_)));

        let stored: Vec<_> = std::fs::read_dir(
            dir.path()
                .join("artifacts")
                .join(run_id.to_string())
                .join(step_id.to_string()),
        )
        .expect("read dir")
        .filter_map(Result::ok)
        .collect();
        assert_eq!(stored.len(), 1, "the rejected blob was not cleaned up");
    }
}
