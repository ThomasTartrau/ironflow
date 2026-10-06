//! Claude Code session of an agent step for [`WorkflowContext`].
//!
//! Before an agent step launches, the engine fixes the id of the session it
//! runs in and records it on the step. When the worker running the step loses
//! its lease, the step is interrupted with [`STEP_INTERRUPTED_ERROR`]; the
//! next execution of the same step resumes that session instead of starting
//! the agent from scratch.

use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use ironflow_store::models::{StepKind, StepUpdate};
use ironflow_store::store::STEP_INTERRUPTED_ERROR;

use crate::config::{DEFAULT_RESUME_PROMPT, StepConfig};
use crate::error::EngineError;
use crate::notify::{WorkflowAgentStepResumedEvent, WorkflowEvent};

use super::WorkflowContext;

impl WorkflowContext {
    /// Fix the Claude Code session an agent step runs in, and record it on
    /// the step before the agent launches.
    ///
    /// When the previous execution of the same step (same attempt, position
    /// and name) was interrupted by a lost lease and recorded a session, the
    /// step resumes it with its resume prompt, [`DEFAULT_RESUME_PROMPT`] when
    /// the author set none. Otherwise the step gets a new session id.
    ///
    /// Does nothing for a step that is not an agent step, for a provider that
    /// cannot pin a session, or when the author already set a session.
    pub(super) async fn assign_agent_session(
        &self,
        config: &mut StepConfig,
        step_id: Uuid,
        position: u32,
        name: &str,
    ) -> Result<(), EngineError> {
        let StepConfig::Agent(agent_config) = config else {
            return Ok(());
        };
        if agent_config.resume_session_id.is_some()
            || agent_config.session_id.is_some()
            || !self.provider.supports_sessions_for(agent_config)
        {
            return Ok(());
        }

        let steps = self.store.list_steps(self.run_id).await?;
        let interrupted_session = steps
            .iter()
            .filter(|step| {
                step.attempt == self.attempt
                    && step.position == position
                    && step.name == name
                    && step.kind == StepKind::Agent
                    && step.error.as_deref() == Some(STEP_INTERRUPTED_ERROR)
                    && step.session_id.is_some()
            })
            .max_by_key(|step| step.created_at)
            .and_then(|step| step.session_id.clone());

        let session_id = match interrupted_session {
            Some(session_id) => {
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    session_id = %session_id,
                    "resuming interrupted agent step from its session"
                );
                agent_config.resume_session_id = Some(session_id.clone());
                agent_config
                    .resume_prompt
                    .get_or_insert_with(|| DEFAULT_RESUME_PROMPT.to_string());
                if let Some(ref bus) = self.event_bus {
                    bus.publish(
                        self.run_id,
                        WorkflowEvent::AgentStepResumed(WorkflowAgentStepResumedEvent {
                            step_name: name.to_string(),
                            step_index: position,
                            session_id: session_id.clone(),
                            timestamp: Utc::now(),
                        }),
                    );
                }
                session_id
            }
            None => {
                let session_id = Uuid::now_v7().to_string();
                agent_config.session_id = Some(session_id.clone());
                session_id
            }
        };

        self.store
            .update_step(
                step_id,
                StepUpdate {
                    session_id: Some(session_id),
                    ..StepUpdate::default()
                },
            )
            .await?;
        Ok(())
    }
}

/// Config a retry of an agent step runs with: the retry resumes the session
/// the step recorded instead of creating it again.
///
/// The first try already created the session pinned by
/// `WorkflowContext::assign_agent_session`, and the CLI refuses to create a
/// session whose id is in use. The retry sends the original prompt into that
/// session; when the first try died before the session existed, the executor
/// falls back to creating it under the same id. The step keeps one session
/// id across its retries. Any other config is returned unchanged.
pub(super) fn retry_in_session(config: &StepConfig) -> StepConfig {
    let mut retry = config.clone();
    if let StepConfig::Agent(agent_config) = &mut retry
        && agent_config.resume_session_id.is_none()
        && let Some(session_id) = agent_config.session_id.take()
    {
        agent_config.resume_session_id = Some(session_id);
        if agent_config.resume_prompt.is_none() {
            agent_config.resume_prompt = Some(agent_config.prompt.clone());
        }
    }
    retry
}
