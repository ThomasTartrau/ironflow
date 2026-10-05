//! `POST /api/v1/runs/:id/steps/:step_id/input` -- Answer a human input step.
//!
//! `POST /api/v1/runs/:id/steps/:step_id/reject` -- Reject a human input step.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::config::HUMAN_INPUT_SCHEMA_KEY;
use ironflow_engine::engine::ExecutionMode;
use ironflow_store::models::{
    Run, RunStatus, Step, StepApproval, StepKind, StepStatus, StepUpdate,
};
use jsonschema::validator_for;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::spawn;
use tracing::error;
use uuid::Uuid;

use crate::entities::RunResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::routes::approve_run::authorize_gate;
use crate::state::AppState;

/// Request body for rejecting a human input step.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct RejectHumanInputRequest {
    /// Why the input is refused. Passed to the handler.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Answer a human input step with a value matching its JSON schema.
///
/// The answer is validated against the schema stored on the step. The caller
/// must be allowed to resolve the step like an approval gate (admin, approver
/// groups, assignee or delegation). The first valid answer wins: the step
/// completes, the run moves from `AwaitingApproval` back to `Running` and
/// resumes, and the handler receives the typed answer.
///
/// # Errors
///
/// - 400 if the step is not a human input or is not awaiting input
/// - 403 if the caller may not answer the step
/// - 404 if the run or the step does not exist
/// - 409 if the input was already answered or rejected
/// - 422 if the answer does not match the schema
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/steps/{step_id}/input",
        tags = ["runs"],
        params(
            ("id" = Uuid, Path, description = "Run ID"),
            ("step_id" = Uuid, Path, description = "Step ID")
        ),
        request_body(content = Value, content_type = "application/json", description = "Answer matching the JSON schema stored on the step"),
        responses(
            (status = 200, description = "Answer recorded, the run resumes", body = RunResponse),
            (status = 400, description = "Step is not a human input awaiting input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run or step not found"),
            (status = 409, description = "Input already answered or rejected"),
            (status = 422, description = "Answer does not match the schema")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn submit_human_input(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((id, step_id)): Path<(Uuid, Uuid)>,
    Json(answer): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let (run, step) = open_input_step(&state, id, step_id).await?;
    let actor = authorize_gate(&auth, &state, &run, &step).await?;
    validate_answer(step.input.as_ref(), &answer)?;

    // Like approve, two concurrent answers are not serialized: both may pass
    // the status check above before either is written. This is an accepted
    // limitation of the gate routes.
    let now = Utc::now();
    state
        .store
        .record_step_approval(
            step.id,
            StepApproval {
                user_id: auth.user_id,
                approved_by: actor,
                at: now,
            },
        )
        .await?;
    state
        .store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                output: Some(answer),
                completed_at: Some(now),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await?;
    state
        .store
        .update_run_status(id, resume_status(&state))
        .await?;

    resume_in_background(&state, id);

    Ok(ok(RunResponse::from(state.get_run_or_404(id).await?)))
}

/// Reject a human input step.
///
/// The step is marked `Rejected` with the reason, the run moves from
/// `AwaitingApproval` back to `Running` and resumes: the handler receives a
/// `HumanInputRejected` error and decides what happens next. The body is
/// optional; without a reason, one naming the caller is recorded.
///
/// # Errors
///
/// - 400 if the step is not a human input or is not awaiting input
/// - 403 if the caller may not resolve the step
/// - 404 if the run or the step does not exist
/// - 409 if the input was already answered or rejected
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/steps/{step_id}/reject",
        tags = ["runs"],
        params(
            ("id" = Uuid, Path, description = "Run ID"),
            ("step_id" = Uuid, Path, description = "Step ID")
        ),
        request_body(content = RejectHumanInputRequest, description = "Optional rejection reason"),
        responses(
            (status = 200, description = "Input rejected, the run resumes", body = RunResponse),
            (status = 400, description = "Step is not a human input awaiting input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run or step not found"),
            (status = 409, description = "Input already answered or rejected")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn reject_human_input(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((id, step_id)): Path<(Uuid, Uuid)>,
    body: Option<Json<RejectHumanInputRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let (run, step) = open_input_step(&state, id, step_id).await?;
    let actor = authorize_gate(&auth, &state, &run, &step).await?;

    let reason = body
        .and_then(|Json(b)| b.reason)
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| format!("input rejected by {actor}"));

    state
        .store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Rejected),
                error: Some(reason),
                completed_at: Some(Utc::now()),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await?;
    state
        .store
        .update_run_status(id, resume_status(&state))
        .await?;

    resume_in_background(&state, id);

    Ok(ok(RunResponse::from(state.get_run_or_404(id).await?)))
}

/// Load the run and the human input step, and check the step can be resolved.
///
/// A resolved step returns 409 whatever the run status, so replaying an
/// accepted request is reported as a conflict rather than a bad request.
async fn open_input_step(
    state: &AppState,
    run_id: Uuid,
    step_id: Uuid,
) -> Result<(Run, Step), ApiError> {
    let run = state.get_run_or_404(run_id).await?;
    let step = state
        .store
        .get_step(step_id)
        .await?
        .filter(|s| s.run_id == run_id)
        .ok_or(ApiError::StepNotFound(step_id))?;

    if step.kind != StepKind::HumanInput {
        return Err(ApiError::BadRequest(
            "step does not wait for input".to_string(),
        ));
    }

    match step.status.state {
        StepStatus::Completed => {
            return Err(ApiError::Conflict("input already provided".to_string()));
        }
        StepStatus::Rejected => {
            return Err(ApiError::Conflict("input already rejected".to_string()));
        }
        _ => {}
    }

    if step.status.state != StepStatus::AwaitingApproval
        || run.status.state != RunStatus::AwaitingApproval
    {
        return Err(ApiError::BadRequest(
            "step is not awaiting input".to_string(),
        ));
    }

    Ok((run, step))
}

/// Validate `answer` against the JSON schema stored in the step input.
fn validate_answer(input: Option<&Value>, answer: &Value) -> Result<(), ApiError> {
    let schema = input
        .and_then(|i| i.get(HUMAN_INPUT_SCHEMA_KEY))
        .ok_or_else(|| ApiError::Internal("input step has no stored schema".to_string()))?;
    let validator = validator_for(schema)
        .map_err(|e| ApiError::Internal(format!("input step has an invalid schema: {e}")))?;

    let errors: Vec<String> = validator
        .iter_errors(answer)
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::InvalidInput(errors))
    }
}

/// The status a run moves to once its input is answered or rejected:
/// `Running` to resume it here, `Pending` to requeue it for a worker.
fn resume_status(state: &AppState) -> RunStatus {
    match state.engine.execution_mode() {
        ExecutionMode::Local => RunStatus::Running,
        ExecutionMode::Workers => RunStatus::Pending,
    }
}

/// Under [`ExecutionMode::Local`], resume the run in the background; the
/// handler replays up to the input. Under [`ExecutionMode::Workers`], do
/// nothing: the run is already `Pending` and a worker picks it up.
fn resume_in_background(state: &AppState, id: Uuid) {
    if !matches!(state.engine.execution_mode(), ExecutionMode::Local) {
        return;
    }
    let engine = state.engine.clone();
    spawn(async move {
        if let Err(err) = engine.resume_run(id).await {
            error!(run_id = %id, error = %err, "failed to resume run after human input");
        }
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::input_schema_for;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{
        Assignee, NewRun, NewStep, NewUser, TriggerKind, User, step_trace_id,
    };
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
    use schemars::JsonSchema;
    use serde_json::{from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use super::*;
    use crate::routes::test_helpers::create_user_auth_header;

    /// The answer type the test steps ask for.
    #[allow(dead_code)]
    #[derive(Deserialize, JsonSchema)]
    struct Answers {
        answers: Vec<String>,
    }

    /// An answer type whose schema carries a `$defs` reference.
    #[allow(dead_code)]
    #[derive(Deserialize, JsonSchema)]
    struct Nested {
        target: Target,
    }

    #[allow(dead_code)]
    #[derive(Deserialize, JsonSchema)]
    struct Target {
        env: String,
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

    fn member_header(user: &User, state: &AppState) -> String {
        let token = AccessToken::for_user(user.id, &user.username, false, &state.jwt_config)
            .expect("token");
        format!("Bearer {}", token.0)
    }

    async fn awaiting_run(store: &Arc<InMemoryStore>) -> Uuid {
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
                concurrency_key: None,
                concurrency_limits: Vec::new(),
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
        run.id
    }

    /// A step of `kind` awaiting approval on `run_id`.
    async fn awaiting_step(
        store: &Arc<InMemoryStore>,
        run_id: Uuid,
        kind: StepKind,
        assignee: Option<Assignee>,
    ) -> Uuid {
        let step = store
            .create_step(NewStep {
                run_id,
                trace_id: step_trace_id(run_id, "clarify", 0),
                name: "clarify".to_string(),
                kind,
                position: 0,
                input: Some(json!({
                    "message": "Answer the questions",
                    HUMAN_INPUT_SCHEMA_KEY: input_schema_for::<Answers>(),
                })),
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
                    ..StepUpdate::default()
                },
            )
            .await
            .unwrap();
        step.id
    }

    /// A run awaiting a human input step, with an admin header.
    async fn setup() -> (Arc<InMemoryStore>, AppState, Uuid, Uuid) {
        let store = Arc::new(InMemoryStore::new());
        let run_id = awaiting_run(&store).await;
        let step_id = awaiting_step(&store, run_id, StepKind::HumanInput, None).await;
        let state = test_state(store.clone());
        (store, state, run_id, step_id)
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .route("/{id}/steps/{step_id}/input", post(submit_human_input))
            .route("/{id}/steps/{step_id}/reject", post(reject_human_input))
            .with_state(state)
    }

    /// POST `body` (or nothing) to `/{run_id}/steps/{step_id}/{verb}`.
    async fn call(
        state: &AppState,
        auth: &str,
        run_id: Uuid,
        step_id: Uuid,
        verb: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method("POST")
            .uri(format!("/{run_id}/steps/{step_id}/{verb}"))
            .header("authorization", auth);
        let req = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };

        let resp = app(state.clone()).oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val = if bytes.is_empty() {
            Value::Null
        } else {
            from_slice(&bytes).unwrap()
        };
        (status, json_val)
    }

    #[tokio::test]
    async fn human_input_submit_valid_answer_resumes_the_run() {
        let (store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;
        let answer = json!({"answers": ["staging"]});

        let (status, body) = call(
            &state,
            &auth,
            run_id,
            step_id,
            "input",
            Some(answer.clone()),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["status"], "running");
        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::Completed);
        assert_eq!(step.output, Some(answer));
        assert_eq!(step.approvals.len(), 1);
        assert_eq!(step.approvals[0].approved_by, "admin");
    }

    #[tokio::test]
    async fn human_input_submit_invalid_answer_returns_422() {
        let (store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;

        for answer in [json!({"answers": 3}), json!({})] {
            let (status, body) = call(&state, &auth, run_id, step_id, "input", Some(answer)).await;

            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(body["error"]["code"], "INVALID_INPUT");
            let errors = body["error"]["details"]["errors"]
                .as_array()
                .expect("errors array");
            assert!(!errors.is_empty());
        }

        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::AwaitingApproval);
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);
    }

    #[tokio::test]
    async fn human_input_submit_twice_returns_409() {
        let (_store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;
        let answer = json!({"answers": ["yes"]});

        let (first, _) = call(
            &state,
            &auth,
            run_id,
            step_id,
            "input",
            Some(answer.clone()),
        )
        .await;
        assert_eq!(first, StatusCode::OK);

        let (second, body) = call(&state, &auth, run_id, step_id, "input", Some(answer)).await;
        assert_eq!(second, StatusCode::CONFLICT);
        assert_eq!(body["error"]["message"], "input already provided");
    }

    #[tokio::test]
    async fn human_input_submit_on_an_approval_step_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run_id = awaiting_run(&store).await;
        let step_id = awaiting_step(&store, run_id, StepKind::Approval, None).await;
        let state = test_state(store);
        let auth = create_user_auth_header(&state, "admin", true).await;

        let answer = json!({"answers": []});
        let (status, _) = call(&state, &auth, run_id, step_id, "input", Some(answer)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn human_input_submit_on_a_run_not_awaiting_returns_400() {
        let (store, state, run_id, step_id) = setup().await;
        store
            .update_run_status(run_id, RunStatus::Running)
            .await
            .unwrap();
        let auth = create_user_auth_header(&state, "admin", true).await;

        let answer = json!({"answers": []});
        let (status, _) = call(&state, &auth, run_id, step_id, "input", Some(answer)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn human_input_unknown_step_or_step_of_another_run_returns_404() {
        let (store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;
        let answer = json!({"answers": []});

        let unknown = Uuid::now_v7();
        let (status, body) = call(
            &state,
            &auth,
            run_id,
            unknown,
            "input",
            Some(answer.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "STEP_NOT_FOUND");

        let other_run = awaiting_run(&store).await;
        let (status, _) = call(&state, &auth, other_run, step_id, "input", Some(answer)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn human_input_unknown_run_returns_404() {
        let (_store, state, _run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;

        let answer = json!({"answers": []});
        let (status, body) = call(
            &state,
            &auth,
            Uuid::now_v7(),
            step_id,
            "input",
            Some(answer),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "RUN_NOT_FOUND");
    }

    #[tokio::test]
    async fn human_input_non_admin_non_assignee_is_forbidden() {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let bob = member(&store, "bob").await;
        let run_id = awaiting_run(&store).await;
        let assignee = Some(Assignee::user(&alice.username));
        let step_id = awaiting_step(&store, run_id, StepKind::HumanInput, assignee).await;
        let state = test_state(store.clone());

        let answer = json!({"answers": ["yes"]});
        let bob_auth = member_header(&bob, &state);
        let (status, _) = call(
            &state,
            &bob_auth,
            run_id,
            step_id,
            "input",
            Some(answer.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let alice_auth = member_header(&alice, &state);
        let (status, _) = call(&state, &alice_auth, run_id, step_id, "input", Some(answer)).await;
        assert_eq!(status, StatusCode::OK);
        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.approvals[0].user_id, alice.id);
    }

    #[tokio::test]
    async fn human_input_reject_marks_the_step_and_resumes_the_run() {
        let (store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;

        let body = json!({"reason": "out of scope"});
        let (status, resp) = call(&state, &auth, run_id, step_id, "reject", Some(body)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"]["status"], "running");
        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::Rejected);
        assert_eq!(step.error.as_deref(), Some("out of scope"));
        assert!(step.approval_deadline_at.is_none());
    }

    /// A run awaiting a human input assigned to a member, on an engine that
    /// requeues resumed runs for a worker.
    async fn workers_setup() -> (Arc<InMemoryStore>, AppState, Uuid, Uuid, String) {
        let store = Arc::new(InMemoryStore::new());
        let alice = member(&store, "alice").await;
        let run_id = awaiting_run(&store).await;
        let assignee = Some(Assignee::user(&alice.username));
        let step_id = awaiting_step(&store, run_id, StepKind::HumanInput, assignee).await;
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine =
            Engine::new(store.clone(), provider).with_execution_mode(ExecutionMode::Workers);
        let jwt_config = Arc::new(JwtConfig {
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
        let auth = member_header(&alice, &state);
        (store, state, run_id, step_id, auth)
    }

    #[tokio::test]
    async fn human_input_submit_in_workers_mode_requeues_the_run() {
        let (store, state, run_id, step_id, auth) = workers_setup().await;

        let answer = json!({"answers": ["ok"]});
        let (status, body) = call(&state, &auth, run_id, step_id, "input", Some(answer)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["status"], "pending");
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Pending);
        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::Completed);
    }

    #[tokio::test]
    async fn human_input_reject_in_workers_mode_requeues_the_run() {
        let (store, state, run_id, step_id, auth) = workers_setup().await;

        let body = json!({"reason": "out of scope"});
        let (status, resp) = call(&state, &auth, run_id, step_id, "reject", Some(body)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"]["status"], "pending");
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Pending);
        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::Rejected);
    }

    #[tokio::test]
    async fn human_input_reject_without_body_uses_the_default_reason() {
        let (store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;

        let (status, _) = call(&state, &auth, run_id, step_id, "reject", None).await;

        assert_eq!(status, StatusCode::OK);
        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.error.as_deref(), Some("input rejected by admin"));
    }

    #[tokio::test]
    async fn human_input_reject_with_a_blank_reason_uses_the_default_reason() {
        let (store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;

        let body = json!({"reason": "   "});
        let (status, _) = call(&state, &auth, run_id, step_id, "reject", Some(body)).await;

        assert_eq!(status, StatusCode::OK);
        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.error.as_deref(), Some("input rejected by admin"));
    }

    #[tokio::test]
    async fn human_input_reject_after_submit_returns_409() {
        let (_store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;

        let answer = json!({"answers": ["yes"]});
        let (first, _) = call(&state, &auth, run_id, step_id, "input", Some(answer)).await;
        assert_eq!(first, StatusCode::OK);

        let (second, _) = call(&state, &auth, run_id, step_id, "reject", None).await;
        assert_eq!(second, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn human_input_submit_after_reject_returns_409() {
        let (_store, state, run_id, step_id) = setup().await;
        let auth = create_user_auth_header(&state, "admin", true).await;

        let (first, _) = call(&state, &auth, run_id, step_id, "reject", None).await;
        assert_eq!(first, StatusCode::OK);

        let answer = json!({"answers": ["yes"]});
        let (second, body) = call(&state, &auth, run_id, step_id, "input", Some(answer)).await;
        assert_eq!(second, StatusCode::CONFLICT);
        assert_eq!(body["error"]["message"], "input already rejected");
    }

    #[test]
    fn human_input_validate_answer_without_a_schema_is_internal() {
        let err = validate_answer(Some(&json!({"message": "Answer?"})), &json!({}))
            .expect_err("no schema");
        assert!(matches!(err, ApiError::Internal(_)), "got {err:?}");

        let err = validate_answer(None, &json!({})).expect_err("no input");
        assert!(matches!(err, ApiError::Internal(_)), "got {err:?}");
    }

    #[test]
    fn human_input_validate_answer_resolves_defs_references() {
        let schema = input_schema_for::<Nested>();
        assert!(schema.get("$defs").is_some(), "schema: {schema}");
        let input = json!({ HUMAN_INPUT_SCHEMA_KEY: schema });

        let valid = validate_answer(Some(&input), &json!({"target": {"env": "prod"}}));
        assert!(valid.is_ok(), "got {valid:?}");

        let err = validate_answer(Some(&input), &json!({"target": {"env": 1}}))
            .expect_err("wrong nested type");
        match err {
            ApiError::InvalidInput(errors) => assert_eq!(errors.len(), 1, "got {errors:?}"),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }
}
