//! `PUT /api/v1/internal/steps/:id` — Update a step after execution.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;
use uuid::Uuid;

use serde_json::{Value, json};

use ironflow_engine::notify::{ApprovalRequestedEvent, Event, StepCompletedEvent, StepFailedEvent};
use ironflow_store::entities::{StepStatus, StepUpdate};

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
/// requirement is published here, where the audit log subscriber lives.
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

    state.store.update_step(id, update).await?;

    if opens_gate && let Some(step) = state.store.get_step(id).await? {
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
                max_cost_usd: None,
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
                max_cost_usd: None,
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
                rule_index: Some(0),
                condition: Some("payload.amount > 10000".to_string()),
                required_approvers: 2,
                approver_groups: vec!["finance".to_string()],
                evaluated: Vec::new(),
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
}
