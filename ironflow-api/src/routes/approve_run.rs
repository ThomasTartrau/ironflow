//! `POST /api/v1/runs/:id/approve` -- Approve a run awaiting human approval.
//!
//! `POST /api/v1/runs/:id/reject` -- Reject a run awaiting human approval.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::{AuthMethod, Authenticated};
use ironflow_engine::notify::{ApprovalGrantedEvent, ApprovalRejectedEvent, Event};
use ironflow_store::models::{Assignee, Run, RunStatus, Step, StepStatus, StepUpdate};
use tokio::spawn;
use uuid::Uuid;

use crate::entities::RunResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Approve a run that is awaiting human approval.
///
/// Transitions the run from `AwaitingApproval` back to `Running`.
/// Returns 400 if the run is not in `AwaitingApproval` state.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/approve",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 200, description = "Run approved successfully", body = RunResponse),
            (status = 400, description = "Run not awaiting approval"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn approve_run(
    auth: Authenticated,
    state: State<AppState>,
    path: Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    resolve_approval(auth, state, path, RunStatus::Running, "approve").await
}

/// Reject a run that is awaiting human approval.
///
/// Transitions the run from `AwaitingApproval` to `Failed`.
/// Returns 400 if the run is not in `AwaitingApproval` state.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/reject",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 200, description = "Run rejected successfully", body = RunResponse),
            (status = 400, description = "Run not awaiting approval"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn reject_run(
    auth: Authenticated,
    state: State<AppState>,
    path: Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    resolve_approval(auth, state, path, RunStatus::Failed, "reject").await
}

/// Decide whether the caller may resolve this gate, and under which name the
/// decision is recorded.
///
/// - An admin resolves any gate under their own name.
/// - The user a gate is assigned to resolves it under their own name.
/// - Anyone else gets through only when the gate is assigned to an individual
///   user and the caller holds an active delegation from that user covering
///   this workflow; the decision is then recorded as
///   `"<caller> (delegated from <delegator>)"`.
///
/// Group and unassigned gates stay admin-only: there is no single person to
/// resolve them or to inherit from.
///
/// Identity is compared by user ID, never by name: an API key is named freely
/// by its owner, so its name must not stand in for a username.
///
/// The check lives here rather than in `ironflow-engine` because gate
/// authorization has always been an API-layer concern: the engine never knows
/// who is calling.
async fn authorize_approver(
    auth: &Authenticated,
    state: &AppState,
    run: &Run,
    steps: &[Step],
) -> Result<String, ApiError> {
    let caller = match &auth.method {
        AuthMethod::Jwt { username, .. } => username.clone(),
        AuthMethod::ApiKey { key_name, .. } => key_name.clone(),
    };

    if auth.is_admin() {
        return Ok(caller);
    }

    let gate = steps
        .iter()
        .find(|s| s.status.state == StepStatus::AwaitingApproval)
        .ok_or(ApiError::Forbidden)?;

    let assignee_name = match gate.approval_assignee.as_ref() {
        Some(Assignee::User(name)) => name,
        _ => return Err(ApiError::Forbidden),
    };

    // An assignee naming no known user can be resolved by an admin only.
    let assignee = state
        .store
        .find_user_by_username(assignee_name)
        .await?
        .ok_or(ApiError::Forbidden)?;

    if assignee.id == auth.user_id {
        return Ok(caller);
    }

    // The store drops expired and not-yet-started rows and applies the glob.
    let delegation = state
        .store
        .find_active_delegation(assignee.id, auth.user_id, &run.workflow_name)
        .await?;

    match delegation {
        Some(_) => Ok(format!("{caller} (delegated from {})", assignee.username)),
        None => Err(ApiError::Forbidden),
    }
}

async fn resolve_approval(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    target_status: RunStatus,
    verb: &str,
) -> Result<impl IntoResponse, ApiError> {
    let run = state.get_run_or_404(id).await?;

    if run.status.state != RunStatus::AwaitingApproval {
        return Err(ApiError::BadRequest(format!(
            "cannot {verb} run in {} state, expected AwaitingApproval",
            run.status.state
        )));
    }

    // On rejection, mark the approval step as Rejected so the dashboard reflects it.
    // On approval, the step is transitioned by the replay in resume_run.
    //
    // Either way the SLA timer is disarmed here: the run resumes asynchronously,
    // and the escalator must not claim a gate a human just resolved.
    let steps = state.store.list_steps(id).await?;

    let actor = authorize_approver(&auth, &state, &run, &steps).await?;

    for step in &steps {
        if step.status.state != StepStatus::AwaitingApproval {
            continue;
        }

        let update = if target_status == RunStatus::Failed {
            StepUpdate {
                status: Some(StepStatus::Rejected),
                completed_at: Some(Utc::now()),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            }
        } else {
            StepUpdate {
                clear_approval_deadline: true,
                ..StepUpdate::default()
            }
        };

        state.store.update_step(step.id, update).await?;
    }

    state.store.update_run_status(id, target_status).await?;

    let publisher = state.engine.event_publisher();
    let now = Utc::now();
    if target_status == RunStatus::Running {
        publisher.publish(Event::ApprovalGranted(ApprovalGrantedEvent {
            run_id: id,
            approved_by: actor,
            at: now,
        }));
    } else {
        publisher.publish(Event::ApprovalRejected(ApprovalRejectedEvent {
            run_id: id,
            rejected_by: actor,
            at: now,
        }));
    }

    // On approval, resume the run in the background.
    // The handler is re-executed with step replay: completed steps
    // return cached output, and execution continues from where it
    // stopped.
    if target_status == RunStatus::Running {
        let engine = state.engine.clone();
        spawn(async move {
            if let Err(err) = engine.resume_run(id).await {
                tracing::error!(run_id = %id, error = %err, "failed to resume run after approval");
            }
        });
    }

    let updated = state.get_run_or_404(id).await?;

    Ok(ok(RunResponse::from(updated)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as HttpStatusCode};
    use axum::routing::post;
    use chrono::TimeDelta;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::{AuditLogSubscriber, Event};
    use ironflow_store::approval_delegation_store::ApprovalDelegationStore;
    use ironflow_store::audit_log_store::AuditLogStore;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{
        AuditLogFilter, EventKind, NewApprovalDelegation, NewRun, NewStep, NewUser, RunStatus,
        StepKind, StepStatus, TriggerKind, User, step_trace_id,
    };
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::broadcast;
    use tokio::time::sleep;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(JwtConfig {
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

    async fn create_awaiting_approval_run(
        store: &Arc<InMemoryStore>,
    ) -> ironflow_store::models::Run {
        let run = store
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
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::AwaitingApproval)
            .await
            .unwrap();
        store.get_run(run.id).await.unwrap().unwrap()
    }

    // -- approve --

    #[tokio::test]
    async fn approve_awaiting_approval_run() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_awaiting_approval_run(&store).await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/approve", post(approve_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/approve", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["status"], "running");
    }

    #[tokio::test]
    async fn approve_pending_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
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

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/approve", post(approve_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/approve", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn approve_nonexistent_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/approve", post(approve_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/approve", Uuid::now_v7()))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::NOT_FOUND);
    }

    // -- reject --

    #[tokio::test]
    async fn reject_awaiting_approval_run() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_awaiting_approval_run(&store).await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["status"], "failed");
    }

    #[tokio::test]
    async fn reject_never_consumes_a_retry_attempt() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 3,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::AwaitingApproval)
            .await
            .unwrap();

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        // A human said no: the run is terminal, whatever max_retries says.
        let rejected = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(rejected.status.state, RunStatus::Failed);
        assert_eq!(rejected.retry_count, 0);
        assert!(rejected.scheduled_at.is_none());
        assert!(store.pick_next_pending(None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reject_transitions_approval_step_to_failed() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_awaiting_approval_run(&store).await;

        // Create an approval step in AwaitingApproval state
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "gate", 0),
                name: "gate".to_string(),
                kind: StepKind::Approval,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .unwrap();
        store
            .update_step(
                step.id,
                ironflow_store::models::StepUpdate {
                    status: Some(StepStatus::Running),
                    ..ironflow_store::models::StepUpdate::default()
                },
            )
            .await
            .unwrap();
        store
            .update_step(
                step.id,
                ironflow_store::models::StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    ..ironflow_store::models::StepUpdate::default()
                },
            )
            .await
            .unwrap();

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        let steps = store.list_steps(run.id).await.unwrap();
        assert_eq!(steps[0].status.state, StepStatus::Rejected);
    }

    #[tokio::test]
    async fn reject_pending_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
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

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    /// A run awaiting approval whose gate carries a live SLA deadline.
    async fn run_with_armed_gate(store: &Arc<InMemoryStore>) -> (Uuid, Uuid) {
        let run = create_awaiting_approval_run(store).await;
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "gate", 0),
                name: "gate".to_string(),
                kind: StepKind::Approval,
                position: 0,
                input: None,
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
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    approval_deadline_at: Some(Utc::now() + TimeDelta::seconds(3600)),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        (run.id, step.id)
    }

    /// Hit `/{id}/{verb}` and return the HTTP status.
    async fn resolve(store: Arc<InMemoryStore>, run_id: Uuid, verb: &str) -> HttpStatusCode {
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/approve", post(approve_run))
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{run_id}/{verb}"))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn approve_clears_the_approval_deadline() {
        let store = Arc::new(InMemoryStore::new());
        let (run_id, step_id) = run_with_armed_gate(&store).await;

        assert_eq!(
            resolve(store.clone(), run_id, "approve").await,
            HttpStatusCode::OK
        );

        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert!(
            step.approval_deadline_at.is_none(),
            "an approved gate must never be escalated afterwards"
        );
    }

    #[tokio::test]
    async fn reject_clears_the_approval_deadline() {
        let store = Arc::new(InMemoryStore::new());
        let (run_id, step_id) = run_with_armed_gate(&store).await;

        assert_eq!(
            resolve(store.clone(), run_id, "reject").await,
            HttpStatusCode::OK
        );

        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::Rejected);
        assert!(step.approval_deadline_at.is_none());
    }

    // -- delegation --

    /// An `AppState` whose engine persists every event to the store's audit
    /// log, so a test can read back the recorded `approved_by`.
    fn test_state_with_audit_log(store: Arc<InMemoryStore>) -> AppState {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    /// Create a non-admin user in the store.
    async fn member(store: &Arc<InMemoryStore>, username: &str) -> User {
        let password_hash = password::hash("password123").expect("hash");
        store
            .create_user(NewUser {
                email: format!("{username}@example.com"),
                username: username.to_string(),
                password_hash,
                // The first user would otherwise become an implicit admin.
                is_admin: Some(false),
            })
            .await
            .expect("create user")
    }

    /// A `Bearer` header for a non-admin session bound to `user`.
    fn member_header(user: &User, state: &AppState) -> String {
        let username = user.username.as_str();
        let token =
            AccessToken::for_user(user.id, username, false, &state.jwt_config).expect("token");
        format!("Bearer {}", token.0)
    }

    /// A run of `workflow_name` awaiting approval on a gate assigned to `assignee`.
    async fn run_with_gate_assigned_to(
        store: &Arc<InMemoryStore>,
        workflow_name: &str,
        assignee: Option<Assignee>,
    ) -> Uuid {
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: workflow_name.to_string(),
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
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::AwaitingApproval)
            .await
            .unwrap();

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "gate", 0),
                name: "gate".to_string(),
                kind: StepKind::Approval,
                position: 0,
                input: None,
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
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    approval_assignee: assignee,
                    approval_deadline_at: Some(Utc::now() + TimeDelta::seconds(3600)),
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();

        run.id
    }

    /// Grant bob an active delegation from alice, optionally narrowed by a glob.
    async fn delegate(
        store: &Arc<InMemoryStore>,
        from: &User,
        to: &User,
        workflow_filter: Option<&str>,
    ) {
        let now = Utc::now();
        store
            .create_delegation(NewApprovalDelegation {
                from_user_id: from.id,
                to_user_id: to.id,
                valid_from: now - TimeDelta::hours(1),
                valid_until: now + TimeDelta::hours(1),
                workflow_filter: workflow_filter.map(str::to_string),
            })
            .await
            .expect("create delegation");
    }

    /// Hit `/{id}/{verb}` with an explicit authorization header.
    async fn resolve_as(state: AppState, auth: &str, run_id: Uuid, verb: &str) -> HttpStatusCode {
        let app = Router::new()
            .route("/{id}/approve", post(approve_run))
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{run_id}/{verb}"))
            .header("content-type", "application/json")
            .header("authorization", auth)
            .body(Body::from("{}"))
            .unwrap();

        app.oneshot(req).await.unwrap().status()
    }

    /// Wait for the audit log to hold an entry of `kind` for `run_id`.
    ///
    /// The publisher dispatches to its subscribers on a spawned task, so the
    /// entry lands shortly after the HTTP response.
    async fn await_audit_payload(
        store: &Arc<InMemoryStore>,
        run_id: Uuid,
        kind: EventKind,
    ) -> JsonValue {
        for _ in 0..100 {
            let page = store
                .list_audit_logs(
                    AuditLogFilter {
                        event_type: Some(kind),
                        run_id: Some(run_id),
                        ..AuditLogFilter::default()
                    },
                    1,
                    10,
                )
                .await
                .expect("list audit logs");
            if let Some(entry) = page.items.first() {
                return entry.payload.clone();
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("no {kind} audit entry was recorded for run {run_id}");
    }

    #[tokio::test]
    async fn delegate_can_approve_gate_assigned_to_delegator() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;
        delegate(&store, &alice, &bob, None).await;

        let state = test_state(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::OK
        );

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Running);
    }

    #[tokio::test]
    async fn delegated_approval_is_recorded_with_the_delegator() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;
        delegate(&store, &alice, &bob, Some("deploy*")).await;

        let state = test_state_with_audit_log(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::OK
        );

        let payload = await_audit_payload(&store, run_id, EventKind::ApprovalGranted).await;
        assert_eq!(payload["approved_by"], "bob (delegated from alice)");
    }

    #[tokio::test]
    async fn delegate_can_reject_on_behalf_of_the_delegator() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;
        delegate(&store, &alice, &bob, None).await;

        let state = test_state_with_audit_log(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "reject").await,
            HttpStatusCode::OK
        );

        let steps = store.list_steps(run_id).await.unwrap();
        assert_eq!(steps[0].status.state, StepStatus::Rejected);

        let payload = await_audit_payload(&store, run_id, EventKind::ApprovalRejected).await;
        assert_eq!(payload["rejected_by"], "bob (delegated from alice)");
    }

    #[tokio::test]
    async fn non_admin_without_delegation_is_forbidden() {
        let store = Arc::new(InMemoryStore::new());
        let _alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;

        let state = test_state(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::FORBIDDEN
        );
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);
    }

    #[tokio::test]
    async fn expired_delegation_does_not_grant_approval() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;

        let now = Utc::now();
        store
            .create_delegation(NewApprovalDelegation {
                from_user_id: alice.id,
                to_user_id: bob.id,
                valid_from: now - TimeDelta::days(10),
                valid_until: now - TimeDelta::days(3),
                workflow_filter: None,
            })
            .await
            .expect("create expired delegation");

        let state = test_state(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn delegation_with_non_matching_workflow_filter_is_forbidden() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;
        delegate(&store, &alice, &bob, Some("cleanup-*")).await;

        let state = test_state(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn delegation_does_not_apply_to_a_group_gate() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::group("sre"))).await;
        delegate(&store, &alice, &bob, None).await;

        let state = test_state(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn delegation_does_not_apply_to_an_unassigned_gate() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id = run_with_gate_assigned_to(&store, "deploy", None).await;
        delegate(&store, &alice, &bob, None).await;

        let state = test_state(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn assignee_approves_own_gate_without_delegation() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;

        let state = test_state_with_audit_log(store.clone());
        let auth = member_header(&alice, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::OK
        );

        let payload = await_audit_payload(&store, run_id, EventKind::ApprovalGranted).await;
        assert_eq!(payload["approved_by"], "alice");
    }

    #[tokio::test]
    async fn assignee_rejects_own_gate_without_delegation() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;

        let state = test_state(store.clone());
        let auth = member_header(&alice, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "reject").await,
            HttpStatusCode::OK
        );

        let steps = store.list_steps(run_id).await.unwrap();
        assert_eq!(steps[0].status.state, StepStatus::Rejected);
    }

    #[tokio::test]
    async fn assignee_is_matched_by_user_id_not_by_caller_name() {
        let store = Arc::new(InMemoryStore::new());
        let _alice = member(&store, "alice").await;
        let carol = member(&store, "carol").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;

        let state = test_state(store.clone());
        // Carol's identity carrying alice's name, as an API key named "alice" would.
        let token =
            AccessToken::for_user(carol.id, "alice", false, &state.jwt_config).expect("token");
        let auth = format!("Bearer {}", token.0);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn gate_assigned_to_an_unknown_user_is_admin_only() {
        let store = Arc::new(InMemoryStore::new());
        let bob = member(&store, "bob").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("ghost"))).await;

        let state = test_state(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn delegation_from_someone_else_does_not_cover_the_gate() {
        let store = Arc::new(InMemoryStore::new());
        let _alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let carol = member(&store, "carol").await;
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;
        delegate(&store, &carol, &bob, None).await;

        let state = test_state(store.clone());
        let auth = member_header(&bob, &state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn admin_still_approves_without_any_delegation() {
        let store = Arc::new(InMemoryStore::new());
        let run_id =
            run_with_gate_assigned_to(&store, "deploy", Some(Assignee::user("alice"))).await;

        let state = test_state_with_audit_log(store.clone());
        // `make_auth_header` mints an admin token, as every pre-existing test uses.
        let auth = make_auth_header(&state);

        assert_eq!(
            resolve_as(state, &auth, run_id, "approve").await,
            HttpStatusCode::OK
        );

        let payload = await_audit_payload(&store, run_id, EventKind::ApprovalGranted).await;
        assert_eq!(
            payload["approved_by"], "testuser",
            "an admin approves under their own name"
        );
    }

    #[tokio::test]
    async fn reject_nonexistent_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/reject", post(reject_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/reject", Uuid::now_v7()))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::NOT_FOUND);
    }
}
