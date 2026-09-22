//! Artifact plumbing for [`WorkflowContext`].
//!
//! Covers the explicit [`put_artifact`](WorkflowContext::put_artifact) and
//! [`get_artifact`](WorkflowContext::get_artifact) API plus the declarative
//! input/output handling the step lifecycle calls around every shell step.

use std::sync::Arc;

use futures_util::StreamExt;
use tracing::warn;
use uuid::Uuid;

use ironflow_artifacts::name::guess_content_type;
use ironflow_artifacts::stream_from_bytes;
use ironflow_store::entities::Artifact;
use ironflow_store::models::ArtifactLookup;

use crate::artifact::{
    ArtifactSink, ArtifactUpload, StepLocation, collect_outputs, materialize_inputs,
};
use crate::config::StepConfig;
use crate::error::EngineError;

use super::WorkflowContext;

impl WorkflowContext {
    /// The artifact backend, or an explicit error when none is configured.
    fn artifact_sink(&self) -> Result<&Arc<dyn ArtifactSink>, EngineError> {
        self.artifact_sink.as_ref().ok_or_else(|| {
            EngineError::ArtifactsUnavailable(
                "no artifact storage is attached to this run".to_string(),
            )
        })
    }

    /// Store an in-memory payload as an artifact of the given step.
    ///
    /// The declarative [`ShellConfig::output`](crate::config::ShellConfig::output)
    /// covers shell steps; this covers custom operations and agent steps, which
    /// have no working directory to collect from.
    ///
    /// The MIME type is guessed from `name` unless `content_type` is set.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ArtifactsUnavailable`] when no backend is
    /// attached, [`EngineError::Artifact`] when the name is invalid or storage
    /// fails, and [`EngineError::Store`] when the step already owns that name.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use uuid::Uuid;
    ///
    /// # async fn example(ctx: &WorkflowContext, step_id: Uuid) -> Result<(), EngineError> {
    /// let artifact = ctx
    ///     .put_artifact(step_id, "summary.json", None, br#"{"ok":true}"#.to_vec())
    ///     .await?;
    /// assert_eq!(artifact.content_type, "application/json");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn put_artifact(
        &self,
        step_id: Uuid,
        name: &str,
        content_type: Option<&str>,
        content: Vec<u8>,
    ) -> Result<Artifact, EngineError> {
        let sink = self.artifact_sink()?;
        sink.put(
            ArtifactUpload {
                run_id: self.run_id,
                step_id,
                name: name.to_string(),
                content_type: content_type
                    .map(str::to_string)
                    .unwrap_or_else(|| guess_content_type(name)),
            },
            stream_from_bytes(content),
        )
        .await
    }

    /// Read back an artifact produced earlier in this run.
    ///
    /// Resolution follows the same rule as a declared input: same run and
    /// attempt, steps positioned strictly before the current one, closest
    /// producer wins.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ArtifactNotFound`] when nothing matches,
    /// [`EngineError::ArtifactsUnavailable`] when no backend is attached, and
    /// [`EngineError::Artifact`] when the bytes cannot be read.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &WorkflowContext) -> Result<(), EngineError> {
    /// let bytes = ctx.get_artifact("build", "report.html").await?;
    /// println!("{} bytes", bytes.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_artifact(&self, step: &str, name: &str) -> Result<Vec<u8>, EngineError> {
        let sink = self.artifact_sink()?;

        let artifact = self
            .store
            .find_artifact_for_input(ArtifactLookup {
                run_id: self.run_id,
                attempt: self.attempt,
                before_position: self.position,
                step_name: step.to_string(),
                name: name.to_string(),
            })
            .await?
            .ok_or_else(|| EngineError::ArtifactNotFound {
                step: step.to_string(),
                name: name.to_string(),
            })?;

        let mut content = sink.get(&artifact).await?;
        let mut buffer = Vec::with_capacity(artifact.size_bytes as usize);
        while let Some(chunk) = content.next().await {
            let chunk = chunk?;
            buffer.extend_from_slice(chunk.as_ref());
        }

        Ok(buffer)
    }

    /// Place a shell step's declared inputs in its working directory.
    ///
    /// A step that declares none needs no backend, so the check for one only
    /// happens when there is something to materialize.
    pub(super) async fn prepare_step_inputs(
        &self,
        config: &StepConfig,
        position: u32,
    ) -> Result<(), EngineError> {
        let StepConfig::Shell(shell) = config else {
            return Ok(());
        };
        if shell.inputs.is_empty() {
            return Ok(());
        }

        materialize_inputs(
            self.artifact_sink()?,
            &self.store,
            shell,
            StepLocation {
                run_id: self.run_id,
                attempt: self.attempt,
                position,
            },
        )
        .await
    }

    /// Store a shell step's declared outputs.
    ///
    /// On a failed step this is best-effort: the collection error is logged and
    /// swallowed so it never masks the failure that actually stopped the step.
    pub(super) async fn store_step_outputs(
        &self,
        config: &StepConfig,
        step_id: Uuid,
        step_name: &str,
        step_succeeded: bool,
    ) -> Result<(), EngineError> {
        let StepConfig::Shell(shell) = config else {
            return Ok(());
        };
        if shell.outputs.is_empty() {
            return Ok(());
        }

        let sink = match self.artifact_sink() {
            Ok(sink) => sink,
            Err(err) if step_succeeded => return Err(err),
            Err(err) => {
                warn!(
                    run_id = %self.run_id,
                    step = %step_name,
                    error = %err,
                    "cannot collect outputs of a failed step"
                );
                return Ok(());
            }
        };

        let collected =
            collect_outputs(sink, shell, self.run_id, step_id, step_name, step_succeeded).await;

        match collected {
            Ok(()) => Ok(()),
            Err(err) if step_succeeded => Err(err),
            Err(err) => {
                warn!(
                    run_id = %self.run_id,
                    step = %step_name,
                    error = %err,
                    "failed to collect outputs of a failed step"
                );
                Ok(())
            }
        }
    }
}
