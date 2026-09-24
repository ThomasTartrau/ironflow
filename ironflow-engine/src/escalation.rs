//! [`ApprovalEscalator`] — resolves approval gates whose SLA deadline expired.
//!
//! An approval gate configured with
//! [`ApprovalConfig::with_deadline`](crate::config::ApprovalConfig::with_deadline)
//! stores its deadline on the step itself, so the timer survives an API or
//! worker restart: nothing is held in memory, and a brand-new process picks the
//! gate up on its next tick.
//!
//! [`ApprovalEscalator::tick`] claims every gate whose deadline has passed and
//! applies the configured [`EscalationPolicy`]. The claim clears the timer in
//! the same transaction, so a deadline fires **at most once** even with several
//! API instances running an escalator. A process that dies between the claim and
//! the escalation leaves the gate open with no timer — the same trade-off the
//! lease reaper accepts.
//!
//! Every firing publishes an [`Event::ApprovalEscalated`], which the
//! [`AuditLogSubscriber`](crate::notify::AuditLogSubscriber) persists with the
//! reason, so the escalation history of a gate is always reconstructable.

use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use reqwest::Client;
use serde_json::json;
use tracing::{error, info, warn};
use uuid::Uuid;

use ironflow_store::models::{Assignee, RunStatus, RunUpdate, Step, StepStatus, StepUpdate};
use strum::IntoStaticStr;

use crate::config::{ApprovalConfig, EscalationPolicy, NotificationTarget};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::notify::{
    ApprovalEscalatedEvent, ApprovalGrantedEvent, ApprovalRejectedEvent, Event, RetryConfig,
    deliver_with_retry, is_success_2xx,
};

/// Actor recorded on a gate resolved by the escalator rather than a human.
pub const SYSTEM_TIMEOUT_ACTOR: &str = "system:timeout";

/// Error recorded on a step and run auto-rejected after an SLA breach.
pub const APPROVAL_TIMEOUT_ERROR: &str = "approval timeout";

/// How many gates a single [`ApprovalEscalator::tick`] resolves.
pub const DEFAULT_ESCALATION_BATCH_SIZE: u32 = 50;

/// What an escalation did to a gate.
///
/// # Examples
///
/// ```
/// use ironflow_engine::escalation::EscalationAction;
///
/// assert_eq!(EscalationAction::Notified(2), EscalationAction::Notified(2));
/// assert_ne!(EscalationAction::Approved, EscalationAction::Rejected);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum EscalationAction {
    /// Gate auto-approved; the run was resumed.
    Approved,
    /// Gate auto-rejected; step and run failed.
    Rejected,
    /// `n` notifications sent; the timer was restarted.
    Notified(usize),
    /// Gate reassigned; the timer was restarted.
    Reassigned(Assignee),
    /// The policy chain ran out; the gate stays open with no timer.
    Exhausted,
    /// The gate resolved between the claim and the escalation — nothing to do.
    Stale,
}

impl EscalationAction {
    /// Short label recorded on the audit entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::escalation::EscalationAction;
    ///
    /// assert_eq!(EscalationAction::Approved.label(), "approved");
    /// assert_eq!(EscalationAction::Notified(3).label(), "notified");
    /// ```
    pub fn label(&self) -> &'static str {
        self.into()
    }
}

/// Outcome of escalating one gate, returned by [`ApprovalEscalator::tick`].
///
/// # Examples
///
/// ```
/// use ironflow_engine::escalation::{EscalationAction, EscalationRecord};
/// use uuid::Uuid;
///
/// let record = EscalationRecord {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     stage: 0,
///     action: EscalationAction::Rejected,
///     reason: "approval deadline of 3600s expired".to_string(),
/// };
/// assert_eq!(record.action, EscalationAction::Rejected);
/// ```
#[derive(Debug, Clone)]
pub struct EscalationRecord {
    /// Run the gate belongs to.
    pub run_id: Uuid,
    /// The approval step that expired.
    pub step_id: Uuid,
    /// Escalation stage that fired (0-based).
    pub stage: u32,
    /// What the escalation did.
    pub action: EscalationAction,
    /// Why it fired.
    pub reason: String,
}

/// Resolves approval gates whose deadline expired.
///
/// Drive it from a periodic loop (the API server ships one) or call
/// [`tick`](Self::tick) directly from a test.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use ironflow_engine::engine::Engine;
/// use ironflow_engine::escalation::ApprovalEscalator;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
///
/// # async fn example() -> Result<(), ironflow_engine::error::EngineError> {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
///
/// let escalator = ApprovalEscalator::new(engine).batch_size(10);
/// let records = escalator.tick().await?;
/// println!("{} gates escalated", records.len());
/// # Ok(())
/// # }
/// ```
pub struct ApprovalEscalator {
    engine: Arc<Engine>,
    client: Client,
    retry: RetryConfig,
    batch_size: u32,
}

impl ApprovalEscalator {
    /// Create an escalator with the default batch size and retry configuration.
    ///
    /// # Panics
    ///
    /// Panics if the TLS backend is unavailable, through
    /// [`RetryConfig::build_client`](crate::notify::RetryConfig::build_client).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::escalation::ApprovalEscalator;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::store::Store;
    ///
    /// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    /// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
    /// let escalator = ApprovalEscalator::new(engine);
    /// ```
    pub fn new(engine: Arc<Engine>) -> Self {
        let retry = RetryConfig::default();
        Self {
            engine,
            client: retry.build_client(),
            retry,
            batch_size: DEFAULT_ESCALATION_BATCH_SIZE,
        }
    }

    /// Set how many gates a single [`tick`](Self::tick) resolves.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::escalation::ApprovalEscalator;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::store::Store;
    ///
    /// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    /// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
    /// let escalator = ApprovalEscalator::new(engine).batch_size(10);
    /// ```
    pub fn batch_size(mut self, batch_size: u32) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Claim and escalate one batch of expired gates.
    ///
    /// A gate that fails to escalate is logged and reported as
    /// [`EscalationAction::Stale`]: one broken gate never aborts the batch.
    ///
    /// An [`EscalationPolicy::AutoApprove`] resumes the run inline, so a long
    /// workflow holds this call until it suspends or finishes. That blocks the
    /// escalator loop, never the HTTP server, and escalations are rare.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] if the batch cannot be claimed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::escalation::ApprovalEscalator;
    ///
    /// # use ironflow_engine::error::EngineError;
    /// # async fn example(escalator: &ApprovalEscalator) -> Result<(), EngineError> {
    /// for record in escalator.tick().await? {
    ///     println!("{} -> {:?}", record.step_id, record.action);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn tick(&self) -> Result<Vec<EscalationRecord>, EngineError> {
        let steps = self
            .engine
            .store()
            .claim_due_approval_deadlines(self.batch_size)
            .await?;

        let mut records = Vec::with_capacity(steps.len());
        for step in &steps {
            match self.escalate(step).await {
                Ok(record) => records.push(record),
                Err(err) => {
                    error!(
                        run_id = %step.run_id,
                        step_id = %step.id,
                        error = %err,
                        "failed to escalate an expired approval gate"
                    );
                    records.push(EscalationRecord {
                        run_id: step.run_id,
                        step_id: step.id,
                        stage: step.approval_stage,
                        action: EscalationAction::Stale,
                        reason: err.to_string(),
                    });
                }
            }
        }

        Ok(records)
    }

    /// Apply the configured policy to one claimed gate.
    async fn escalate(&self, step: &Step) -> Result<EscalationRecord, EngineError> {
        let stage = step.approval_stage;

        let Some(input) = step.input.as_ref() else {
            error!(step_id = %step.id, "approval step has no stored configuration");
            return Ok(self.stale(step, "approval step has no stored configuration"));
        };
        let config: ApprovalConfig = match serde_json::from_value(input.clone()) {
            Ok(config) => config,
            Err(err) => {
                error!(step_id = %step.id, error = %err, "unreadable approval configuration");
                return Ok(self.stale(step, "unreadable approval configuration"));
            }
        };

        // A human may have resolved the gate between the claim and now.
        let run = self.engine.store().get_run(step.run_id).await?;
        let Some(run) = run else {
            return Ok(self.stale(step, "run no longer exists"));
        };
        if run.status.state != RunStatus::AwaitingApproval
            || step.status.state != StepStatus::AwaitingApproval
        {
            return Ok(self.stale(step, "gate already resolved"));
        }

        let policy = config.effective_policy();
        let reason = format!(
            "approval deadline of {}s expired",
            config.effective_deadline_secs().unwrap_or(0)
        );

        let Some(current) = resolve_stage(&policy, stage)? else {
            warn!(
                run_id = %step.run_id,
                step_id = %step.id,
                stage,
                "escalation chain exhausted; the approval gate stays open with no timer"
            );
            self.publish(
                step,
                stage,
                "chain",
                &EscalationAction::Exhausted,
                &reason,
                None,
            );
            return Ok(EscalationRecord {
                run_id: step.run_id,
                step_id: step.id,
                stage,
                action: EscalationAction::Exhausted,
                reason,
            });
        };

        let action = match current {
            EscalationPolicy::AutoApprove => self.auto_approve(step, &reason).await?,
            EscalationPolicy::AutoReject => self.auto_reject(step).await?,
            EscalationPolicy::Notify(targets) => {
                self.notify(step, &config, &policy, targets, &reason)
                    .await?
            }
            EscalationPolicy::Escalate(assignee) => {
                self.reassign(step, &config, &policy, assignee).await?
            }
            // `resolve_stage` already rejected a nested chain.
            EscalationPolicy::Chain(_) => unreachable!("nested chains are rejected upfront"),
        };

        let assignee = match &action {
            EscalationAction::Reassigned(to) => Some(to.clone()),
            _ => step.approval_assignee.clone(),
        };
        self.publish(
            step,
            stage,
            policy_label(current),
            &action,
            &reason,
            assignee,
        );

        Ok(EscalationRecord {
            run_id: step.run_id,
            step_id: step.id,
            stage,
            action,
            reason,
        })
    }

    /// Record for a gate that resolved between the claim and the escalation.
    fn stale(&self, step: &Step, reason: &str) -> EscalationRecord {
        EscalationRecord {
            run_id: step.run_id,
            step_id: step.id,
            stage: step.approval_stage,
            action: EscalationAction::Stale,
            reason: reason.to_string(),
        }
    }

    /// Complete the gate as if a human had approved it, and resume the run.
    async fn auto_approve(
        &self,
        step: &Step,
        reason: &str,
    ) -> Result<EscalationAction, EngineError> {
        let now = Utc::now();
        let store = self.engine.store();

        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(json!({
                        "approved_by": SYSTEM_TIMEOUT_ACTOR,
                        "escalated_at": now,
                        "reason": reason,
                    })),
                    completed_at: Some(now),
                    clear_approval_deadline: true,
                    ..StepUpdate::default()
                },
            )
            .await?;
        store
            .update_run_status(step.run_id, RunStatus::Running)
            .await?;

        self.engine
            .event_publisher()
            .publish(Event::ApprovalGranted(ApprovalGrantedEvent {
                run_id: step.run_id,
                step_id: Some(step.id),
                approved_by: SYSTEM_TIMEOUT_ACTOR.to_string(),
                approvals_received: step.approvals.len() as u32,
                approvals_required: step
                    .approval_requirement
                    .as_ref()
                    .map_or(1, |r| r.required_approvers),
                requirement: step.approval_requirement.clone(),
                at: now,
            }));

        // The state change already happened: a failed resume is reported, not
        // rolled back. The run sits in `Running` without a lease, exactly like
        // the human-approval path.
        if let Err(err) = self.engine.resume_run(step.run_id).await {
            error!(
                run_id = %step.run_id,
                error = %err,
                "failed to resume run after an auto-approved gate"
            );
        }

        Ok(EscalationAction::Approved)
    }

    /// Fail the gate and the run.
    async fn auto_reject(&self, step: &Step) -> Result<EscalationAction, EngineError> {
        let now = Utc::now();
        let store = self.engine.store();

        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Failed),
                    error: Some(APPROVAL_TIMEOUT_ERROR.to_string()),
                    completed_at: Some(now),
                    clear_approval_deadline: true,
                    ..StepUpdate::default()
                },
            )
            .await?;
        store
            .update_run(
                step.run_id,
                RunUpdate {
                    status: Some(RunStatus::Failed),
                    error: Some(APPROVAL_TIMEOUT_ERROR.to_string()),
                    completed_at: Some(now),
                    ..RunUpdate::default()
                },
            )
            .await?;

        self.engine
            .event_publisher()
            .publish(Event::ApprovalRejected(ApprovalRejectedEvent {
                run_id: step.run_id,
                step_id: Some(step.id),
                rejected_by: SYSTEM_TIMEOUT_ACTOR.to_string(),
                requirement: step.approval_requirement.clone(),
                at: now,
            }));

        Ok(EscalationAction::Rejected)
    }

    /// Notify every target, then restart the timer.
    async fn notify(
        &self,
        step: &Step,
        config: &ApprovalConfig,
        policy: &EscalationPolicy,
        targets: &[NotificationTarget],
        reason: &str,
    ) -> Result<EscalationAction, EngineError> {
        let event = self.escalated_event(
            step,
            step.approval_stage,
            "notify",
            EscalationAction::Notified(targets.len()).label(),
            reason,
            step.approval_assignee.clone(),
        );
        self.deliver_notifications(targets, &event).await;

        self.rearm(step, config, policy, None).await?;
        Ok(EscalationAction::Notified(targets.len()))
    }

    /// Reassign the gate, then restart the timer.
    async fn reassign(
        &self,
        step: &Step,
        config: &ApprovalConfig,
        policy: &EscalationPolicy,
        assignee: &Assignee,
    ) -> Result<EscalationAction, EngineError> {
        self.rearm(step, config, policy, Some(assignee)).await?;
        Ok(EscalationAction::Reassigned(assignee.clone()))
    }

    /// Restart the timer for a policy that left the gate open.
    ///
    /// Outside a [`EscalationPolicy::Chain`], a repeating policy re-arms at the
    /// same stage and therefore fires again; inside a chain, the stage advances
    /// so the next link runs at the next expiry.
    async fn rearm(
        &self,
        step: &Step,
        config: &ApprovalConfig,
        policy: &EscalationPolicy,
        assignee: Option<&Assignee>,
    ) -> Result<(), EngineError> {
        let next_stage = next_stage(policy, step.approval_stage);
        let deadline = config
            .effective_deadline_secs()
            .map(|secs| Utc::now() + TimeDelta::seconds(secs as i64));

        self.engine
            .store()
            .update_step(
                step.id,
                StepUpdate {
                    approval_deadline_at: deadline,
                    approval_stage: Some(next_stage),
                    approval_assignee: assignee.cloned(),
                    ..StepUpdate::default()
                },
            )
            .await?;

        Ok(())
    }

    /// Build the escalation event describing one firing.
    fn escalated_event(
        &self,
        step: &Step,
        stage: u32,
        policy: &str,
        action: &str,
        reason: &str,
        assignee: Option<Assignee>,
    ) -> ApprovalEscalatedEvent {
        ApprovalEscalatedEvent {
            run_id: step.run_id,
            step_id: step.id,
            step_name: step.name.clone(),
            stage,
            policy: policy.to_string(),
            action: action.to_string(),
            reason: reason.to_string(),
            assignee,
            at: Utc::now(),
        }
    }

    /// Publish the audit event for one firing.
    fn publish(
        &self,
        step: &Step,
        stage: u32,
        policy: &str,
        action: &EscalationAction,
        reason: &str,
        assignee: Option<Assignee>,
    ) {
        let event = self.escalated_event(step, stage, policy, action.label(), reason, assignee);

        info!(
            run_id = %step.run_id,
            step_id = %step.id,
            stage,
            policy,
            action = action.label(),
            reason,
            "approval gate escalated"
        );

        self.engine
            .event_publisher()
            .publish(Event::ApprovalEscalated(event));
    }

    /// POST the escalation event to every configured target.
    ///
    /// Delivery failures are logged by
    /// [`deliver_with_retry`](crate::notify::deliver_with_retry) and never
    /// propagate: a dead webhook must not block the timer reset.
    async fn deliver_notifications(
        &self,
        targets: &[NotificationTarget],
        event: &ApprovalEscalatedEvent,
    ) {
        for target in targets {
            match target {
                NotificationTarget::Webhook { url } => {
                    deliver_with_retry(
                        &self.retry,
                        || self.client.post(url).json(event),
                        is_success_2xx,
                        "approval_escalation",
                        url,
                    )
                    .await;
                }
                NotificationTarget::Slack {
                    webhook_url,
                    channel,
                } => {
                    let body = json!({
                        "channel": channel,
                        "text": format!(
                            "⏰ Approval gate `{}` on run {} hit its SLA: {}",
                            event.step_name, event.run_id, event.reason
                        ),
                    });
                    deliver_with_retry(
                        &self.retry,
                        || self.client.post(webhook_url).json(&body),
                        is_success_2xx,
                        "approval_escalation",
                        webhook_url,
                    )
                    .await;
                }
            }
        }
    }
}

/// The policy to apply at `stage`, rejecting a misconfigured nested chain.
///
/// Returns `Ok(None)` when the chain is exhausted: the gate stays open with no
/// timer instead of being silently resolved.
fn resolve_stage(
    policy: &EscalationPolicy,
    stage: u32,
) -> Result<Option<&EscalationPolicy>, EngineError> {
    match policy.stage(stage as usize) {
        Some(EscalationPolicy::Chain(_)) => Err(EngineError::StepConfig(
            "nested escalation chains are not supported".to_string(),
        )),
        other => Ok(other),
    }
}

/// The stage to re-arm at after a policy that left the gate open.
///
/// A bare repeating policy stays at the same stage and fires again; a policy
/// inside a chain advances so the next link runs at the next expiry.
fn next_stage(policy: &EscalationPolicy, stage: u32) -> u32 {
    match policy {
        EscalationPolicy::Chain(_) => stage.saturating_add(1),
        _ => stage,
    }
}

/// Wire label of the policy that fired, for the audit entry.
fn policy_label(policy: &EscalationPolicy) -> &'static str {
    match policy {
        EscalationPolicy::AutoApprove => "auto_approve",
        EscalationPolicy::AutoReject => "auto_reject",
        EscalationPolicy::Notify(_) => "notify",
        EscalationPolicy::Escalate(_) => "escalate",
        EscalationPolicy::Chain(_) => "chain",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_compare_by_value() {
        assert_eq!(EscalationAction::Notified(2), EscalationAction::Notified(2));
        assert_ne!(EscalationAction::Notified(2), EscalationAction::Notified(3));
        assert_eq!(
            EscalationAction::Reassigned(Assignee::group("sre")),
            EscalationAction::Reassigned(Assignee::group("sre"))
        );
        assert_ne!(EscalationAction::Approved, EscalationAction::Rejected);
    }

    #[test]
    fn action_labels_are_distinct() {
        let labels = [
            EscalationAction::Approved.label(),
            EscalationAction::Rejected.label(),
            EscalationAction::Notified(1).label(),
            EscalationAction::Reassigned(Assignee::group("sre")).label(),
            EscalationAction::Exhausted.label(),
            EscalationAction::Stale.label(),
        ];

        for (i, a) in labels.iter().enumerate() {
            for (j, b) in labels.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "labels {i} and {j} collide");
                }
            }
        }
    }

    #[test]
    fn a_bare_repeating_policy_stays_at_the_same_stage() {
        let notify = EscalationPolicy::Notify(Vec::new());
        assert_eq!(next_stage(&notify, 0), 0);
        assert_eq!(next_stage(&notify, 7), 7);

        let escalate = EscalationPolicy::Escalate(Assignee::group("sre"));
        assert_eq!(next_stage(&escalate, 3), 3);
    }

    #[test]
    fn a_chained_policy_advances_one_stage_per_expiry() {
        let chain = EscalationPolicy::Chain(vec![
            EscalationPolicy::Notify(Vec::new()),
            EscalationPolicy::AutoReject,
        ]);
        assert_eq!(next_stage(&chain, 0), 1);
        assert_eq!(next_stage(&chain, 1), 2);
        assert_eq!(next_stage(&chain, u32::MAX), u32::MAX);
    }

    #[test]
    fn resolve_stage_rejects_a_nested_chain() {
        let policy = EscalationPolicy::Chain(vec![
            EscalationPolicy::Chain(vec![EscalationPolicy::AutoReject]),
            EscalationPolicy::AutoApprove,
        ]);

        let err = resolve_stage(&policy, 0).expect_err("nested chains are rejected");
        assert!(
            err.to_string().contains("nested escalation chains"),
            "got {err}"
        );

        // The rest of the chain is still usable.
        assert_eq!(
            resolve_stage(&policy, 1).expect("valid stage"),
            Some(&EscalationPolicy::AutoApprove)
        );
    }

    #[test]
    fn resolve_stage_reports_an_exhausted_chain_as_none() {
        let policy = EscalationPolicy::Chain(vec![EscalationPolicy::AutoReject]);
        assert!(resolve_stage(&policy, 1).expect("valid call").is_none());

        let bare = EscalationPolicy::AutoApprove;
        assert!(resolve_stage(&bare, 1).expect("valid call").is_none());
        assert_eq!(
            resolve_stage(&bare, 0).expect("valid call"),
            Some(&EscalationPolicy::AutoApprove)
        );
    }

    #[test]
    fn policy_labels_match_the_wire_format() {
        assert_eq!(policy_label(&EscalationPolicy::AutoApprove), "auto_approve");
        assert_eq!(policy_label(&EscalationPolicy::AutoReject), "auto_reject");
        assert_eq!(
            policy_label(&EscalationPolicy::Notify(Vec::new())),
            "notify"
        );
        assert_eq!(
            policy_label(&EscalationPolicy::Escalate(Assignee::group("sre"))),
            "escalate"
        );
        assert_eq!(policy_label(&EscalationPolicy::Chain(Vec::new())), "chain");
    }
}
