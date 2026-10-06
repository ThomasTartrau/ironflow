//! `PUT /api/v1/internal/steps/:id` — Update a step after execution.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;
use uuid::Uuid;

use serde_json::{Value, json};

use ironflow_engine::notify::{ApprovalRequestedEvent, Event, StepCompletedEvent, StepFailedEvent};
use ironflow_store::entities::{Step, StepKind, StepStatus, StepUpdate};

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Update a step's status, output, and metrics (used by the worker).
///
/// After persisting the update, broadcasts a matching [`Event::StepCompleted`]
/// or [`Event::StepFailed`] so SSE subscribers see step-level progress while
/// the worker is running the pipeline remotely.
///
/// An update moving the step to `AwaitingApproval` means a gate just opened
/// on the worker: an [`Event::ApprovalRequested`] carrying the stored approval
/// requirement is published here, where the audit log subscriber lives. A
/// human input step opens the same way but is not an approval: no event is
/// published for it.
///
/// `environment_id` is produced by the engine running on the worker, which
/// persists it through this route so a replayed agent step hands the same id
/// back. The route only takes a value the engine can have produced: the step
/// must be an agent step, the id must be a PersistentVolumeClaim name, and an
/// id already recorded on the step cannot be replaced by another one.
/// Anything else is refused with `400 Bad Request` before the store is
/// touched.
pub async fn update_step(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(update): Json<StepUpdate>,
) -> Result<impl IntoResponse, ApiError> {
    let terminal_status = update.status;
    let duration_ms = update.duration_ms.unwrap_or(0);
    let cost_usd = update.cost_usd.unwrap_or_default();
    let error_msg = update.error.clone();
    let opens_gate = update.status == Some(StepStatus::AwaitingApproval);

    if let Some(environment_id) = update.environment_id.as_deref() {
        let step = state
            .store
            .get_step(id)
            .await?
            .ok_or(ApiError::StepNotFound(id))?;
        check_environment_id(&step, environment_id)?;
    }

    state.store.update_step(id, update).await?;

    if opens_gate
        && let Some(step) = state.store.get_step(id).await?
        && step.kind != StepKind::HumanInput
    {
        let message = step
            .input
            .as_ref()
            .and_then(|v| v.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        state
            .engine
            .event_publisher()
            .publish(Event::ApprovalRequested(ApprovalRequestedEvent {
                run_id: step.run_id,
                step_id: step.id,
                message,
                requirement: step.approval_requirement.clone(),
                at: Utc::now(),
            }));
    }

    if matches!(
        terminal_status,
        Some(StepStatus::Completed) | Some(StepStatus::Failed)
    ) && let Some(step) = state.store.get_step(id).await?
    {
        let now = Utc::now();
        let event = match terminal_status {
            Some(StepStatus::Completed) => Event::StepCompleted(StepCompletedEvent {
                run_id: step.run_id,
                step_id: step.id,
                step_name: step.name.clone(),
                kind: step.kind.clone(),
                duration_ms,
                cost_usd,
                at: now,
            }),
            Some(StepStatus::Failed) => Event::StepFailed(StepFailedEvent {
                run_id: step.run_id,
                step_id: step.id,
                step_name: step.name.clone(),
                kind: step.kind.clone(),
                error: error_msg.unwrap_or_default(),
                at: now,
            }),
            _ => unreachable!(),
        };
        state.engine.event_publisher().publish(event);
    }

    Ok(ok(json!({ "updated": true })))
}

/// Longest name Kubernetes accepts for a PersistentVolumeClaim.
const ENVIRONMENT_ID_MAX: usize = 253;

/// Refuse an `environment_id` the engine cannot have produced for `step`.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] when `step` is not an agent step, when
/// `environment_id` is not a DNS-1123 name of at most 253 characters, or when
/// `step` already records a different environment.
fn check_environment_id(step: &Step, environment_id: &str) -> Result<(), ApiError> {
    if step.kind != StepKind::Agent {
        return Err(ApiError::BadRequest(format!(
            "environment_id is only recorded on agent steps, step {} is {}",
            step.id, step.kind
        )));
    }
    let valid_name = !environment_id.is_empty()
        && environment_id.len() <= ENVIRONMENT_ID_MAX
        && environment_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid_name {
        return Err(ApiError::BadRequest(format!(
            "environment_id must be 1 to {ENVIRONMENT_ID_MAX} lowercase ASCII letters, digits or '-'"
        )));
    }
    if let Some(recorded) = step.environment_id.as_deref()
        && recorded != environment_id
    {
        return Err(ApiError::BadRequest(format!(
            "step {} already ran in environment '{recorded}'",
            step.id
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::{AuditLogSubscriber, Event};
    use ironflow_store::audit_log_store::AuditLogStore;
    use ironflow_store::entities::{
        ApprovalRequirement, AuditLogFilter, EventKind, NewStep, StepKind, StepStatus,
        step_trace_id,
    };
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::{Value as JsonValue, from_slice, json, to_string};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::broadcast;
    use tokio::time::sleep;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
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

    #[tokio::test]
    async fn update_step_success() {
        let state = test_state();
        let run = state
            .store
            .create_run(NewRun {
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
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run();

        let step = state
            .store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "step1", 0),
                name: "step1".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: Some(json!({"tool": "test"})),
                is_error_handler: false,
            })
            .await
            .unwrap();

        // Transition Pending -> Running first
        state
            .store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        let app = create_router(state.clone(), RouterConfig::default());

        // Now transition Running -> Completed
        let update = StepUpdate {
            status: Some(StepStatus::Completed),
            output: Some(json!({"result": "success"})),
            error: None,
            duration_ms: Some(1000),
            cost_usd: None,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            started_at: None,
            completed_at: None,
            debug_messages: None,
            approval_deadline_at: None,
            approval_stage: None,
            approval_assignee: None,
            approval_requirement: None,
            clear_approval_deadline: false,
            account_id: None,
            environment_id: None,
        };

        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/steps/{}", step.id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&update).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["updated"], true);

        let steps = state.store.list_steps(run.id).await.unwrap();
        let updated = steps
            .iter()
            .find(|s| s.id == step.id)
            .expect("step should exist");
        assert_eq!(updated.status.state, StepStatus::Completed);
        assert_eq!(updated.output, Some(json!({"result": "success"})));
        assert_eq!(updated.duration_ms, 1000);
    }

    #[tokio::test]
    async fn awaiting_approval_update_publishes_approval_requested() {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        );

        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "payments".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({"amount": 15000}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run();
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "finance-gate", 1),
                name: "finance-gate".to_string(),
                kind: StepKind::Approval,
                position: 1,
                input: Some(json!({"message": "Release the payment?"})),
                is_error_handler: false,
            })
            .await
            .unwrap();
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        // What the worker sends when the gate opens.
        let update = StepUpdate {
            status: Some(StepStatus::AwaitingApproval),
            approval_stage: Some(0),
            approval_requirement: Some(ApprovalRequirement {
                reason: Some("amount > 10k".to_string()),
                required_approvers: 2,
                approver_groups: vec!["finance".to_string()],
            }),
            ..StepUpdate::default()
        };

        let app = create_router(state, RouterConfig::default());
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/steps/{}", step.id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&update).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let stored = store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(stored.approval_requirement, update.approval_requirement);

        // The publisher dispatches to subscribers on a spawned task.
        let mut entries = Vec::new();
        for _ in 0..100 {
            entries = store
                .list_audit_logs(
                    AuditLogFilter {
                        event_type: Some(EventKind::ApprovalRequested),
                        run_id: Some(run.id),
                        ..AuditLogFilter::default()
                    },
                    1,
                    10,
                )
                .await
                .unwrap()
                .items;
            if !entries.is_empty() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(entries.len(), 1, "expected one approval_requested entry");
        let payload = &entries[0].payload;
        assert_eq!(entries[0].step_id, Some(step.id));
        assert_eq!(payload["message"], "Release the payment?");
        assert_eq!(payload["requirement"]["required_approvers"], json!(2));
        assert_eq!(
            payload["requirement"]["approver_groups"],
            json!(["finance"])
        );
    }

    #[tokio::test]
    async fn awaiting_approval_update_of_a_human_input_publishes_nothing() {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        );

        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "clarify".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run();
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "clarify", 0),
                name: "clarify".to_string(),
                kind: StepKind::HumanInput,
                position: 0,
                input: Some(json!({"message": "Answer?", "schema": {"type": "object"}})),
                is_error_handler: false,
            })
            .await
            .unwrap();
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        let update = StepUpdate {
            status: Some(StepStatus::AwaitingApproval),
            approval_stage: Some(0),
            ..StepUpdate::default()
        };
        let app = create_router(state, RouterConfig::default());
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/steps/{}", step.id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&update).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let stored = store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(stored.status.state, StepStatus::AwaitingApproval);

        // Leave the publisher's spawned task time to deliver anything it got.
        sleep(Duration::from_millis(200)).await;
        let entries = store
            .list_audit_logs(
                AuditLogFilter {
                    event_type: Some(EventKind::ApprovalRequested),
                    run_id: Some(run.id),
                    ..AuditLogFilter::default()
                },
                1,
                10,
            )
            .await
            .unwrap()
            .items;
        assert!(entries.is_empty(), "got {entries:?}");
    }

    #[tokio::test]
    async fn update_step_not_found() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let fake_id = Uuid::now_v7();
        let update = StepUpdate {
            status: Some(StepStatus::Completed),
            output: Some(json!({"result": "success"})),
            error: None,
            duration_ms: None,
            cost_usd: None,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            started_at: None,
            completed_at: None,
            debug_messages: None,
            approval_deadline_at: None,
            approval_stage: None,
            approval_assignee: None,
            approval_requirement: None,
            clear_approval_deadline: false,
            account_id: None,
            environment_id: None,
        };

        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/steps/{}", fake_id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&update).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// A running step of `kind` in a fresh run of `state`'s store.
    async fn running_step(state: &AppState, kind: StepKind) -> Step {
        let run = state
            .store
            .create_run(NewRun {
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
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run();

        let step = state
            .store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "work", 0),
                name: "work".to_string(),
                kind,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .unwrap();

        state
            .store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        step
    }

    /// Send `update` for `step_id` the way the worker does.
    async fn put_update(state: &AppState, step_id: Uuid, update: &StepUpdate) -> StatusCode {
        let app = create_router(state.clone(), RouterConfig::default());
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/steps/{step_id}"))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(update).unwrap()))
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    fn completed_in(environment_id: &str) -> StepUpdate {
        StepUpdate {
            status: Some(StepStatus::Completed),
            environment_id: Some(environment_id.to_string()),
            ..StepUpdate::default()
        }
    }

    #[tokio::test]
    async fn agent_step_records_the_environment_id_from_the_worker() {
        let state = test_state();
        let step = running_step(&state, StepKind::Agent).await;

        let status = put_update(&state, step.id, &completed_in("ironflow-env-0192f0c1")).await;

        assert_eq!(status, StatusCode::OK);
        let stored = state.store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(stored.status.state, StepStatus::Completed);
        assert_eq!(
            stored.environment_id.as_deref(),
            Some("ironflow-env-0192f0c1")
        );
    }

    #[tokio::test]
    async fn environment_id_on_a_non_agent_step_is_refused() {
        let state = test_state();
        let step = running_step(&state, StepKind::Shell).await;

        let status = put_update(&state, step.id, &completed_in("ironflow-env-0192f0c1")).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        let stored = state.store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(stored.status.state, StepStatus::Running);
        assert!(stored.environment_id.is_none());
    }

    #[tokio::test]
    async fn environment_id_that_is_not_a_pvc_name_is_refused() {
        let state = test_state();
        let step = running_step(&state, StepKind::Agent).await;

        let too_long = "a".repeat(254);
        for bad in ["", "Env-1", "env/../other", "env id", too_long.as_str()] {
            let status = put_update(&state, step.id, &completed_in(bad)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "accepted {bad:?}");
        }

        let stored = state.store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(stored.status.state, StepStatus::Running);
        assert!(stored.environment_id.is_none());
    }

    #[tokio::test]
    async fn recorded_environment_id_cannot_be_replaced() {
        let state = test_state();
        let step = running_step(&state, StepKind::Agent).await;
        let first = StepUpdate {
            environment_id: Some("ironflow-env-first".to_string()),
            ..StepUpdate::default()
        };
        assert_eq!(put_update(&state, step.id, &first).await, StatusCode::OK);

        let status = put_update(&state, step.id, &completed_in("ironflow-env-other")).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        let stored = state.store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(stored.environment_id.as_deref(), Some("ironflow-env-first"));
        assert_eq!(stored.status.state, StepStatus::Running);
    }

    #[tokio::test]
    async fn recorded_environment_id_can_be_sent_again() {
        let state = test_state();
        let step = running_step(&state, StepKind::Agent).await;
        let first = StepUpdate {
            environment_id: Some("ironflow-env-first".to_string()),
            ..StepUpdate::default()
        };
        assert_eq!(put_update(&state, step.id, &first).await, StatusCode::OK);

        let status = put_update(&state, step.id, &completed_in("ironflow-env-first")).await;

        assert_eq!(status, StatusCode::OK);
        let stored = state.store.get_step(step.id).await.unwrap().unwrap();
        assert_eq!(stored.status.state, StepStatus::Completed);
    }

    #[tokio::test]
    async fn environment_id_for_an_unknown_step_is_not_found() {
        let state = test_state();

        let status = put_update(&state, Uuid::now_v7(), &completed_in("ironflow-env-x")).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
