//! `POST /api/v1/runs/:id/replay` -- Replay a finished run on the current handler version.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::{Event, RunCreatedEvent};
use ironflow_engine::replay_policy::is_run_replayable;
use ironflow_store::models::{NewRun, TriggerKind};
use uuid::Uuid;

use crate::actor::run_actor_of;
use crate::entities::RunResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Replay a finished run on the current handler version.
///
/// Creates a new `Pending` run with `TriggerKind::Replay` pointing to the
/// original, always on the current handler version. The original run is not modified.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/replay",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 201, description = "Run replay created successfully", body = RunResponse),
            (status = 400, description = "Run cannot be replayed"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run not found or workflow not registered")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn replay_run(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let original = state.get_run_or_404(id).await?;

    if !is_run_replayable(original.status.state) {
        return Err(ApiError::BadRequest(format!(
            "cannot replay run in {} state",
            original.status.state
        )));
    }

    // A replay always runs on the current handler, so a workflow that is no
    // longer registered cannot be replayed: there is no version to fall back to.
    let Some(handler) = state.engine.get_handler(&original.workflow_name) else {
        return Err(ApiError::WorkflowNotFound(original.workflow_name.clone()));
    };

    let new_run = state
        .store
        .create_run(NewRun {
            workflow_name: original.workflow_name.clone(),
            trigger: TriggerKind::Replay {
                original_run_id: id,
            },
            payload: original.payload,
            max_retries: original.max_retries,
            handler_version: handler.version().map(str::to_string),
            labels: original.labels,
            scheduled_at: None,
            // The replay is attributed to the user who triggered it.
            created_by: Some(run_actor_of(&auth)),
            // A replay is a new logical operation: it must not inherit the
            // original idempotency key.
            idempotency_key: None,
            // Inherit the original cost cap so budget constraints survive replays.
            max_cost_usd: original.max_cost_usd,
        })
        .await?
        .into_run();

    state
        .engine
        .event_publisher()
        .publish(Event::RunCreated(RunCreatedEvent {
            run_id: new_run.id,
            workflow_name: new_run.workflow_name.clone(),
            at: Utc::now(),
        }));

    Ok((StatusCode::CREATED, ok(RunResponse::from(new_run))))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode as HttpStatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{
        NewRun, NewUser, Run, RunActor, RunFilter, RunStatus, TriggerKind,
    };
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
    use rust_decimal::Decimal;
    use serde_json::{Value as JsonValue, from_slice, from_value, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    const WORKFLOW: &str = "replay-wf";

    struct V2Handler;
    impl WorkflowHandler for V2Handler {
        fn name(&self) -> &str {
            WORKFLOW
        }
        fn version(&self) -> Option<&str> {
            Some("2.0.0")
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(V2Handler).unwrap();
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

    /// Create a run and drive it through `path` with legal FSM transitions.
    async fn create_run_in(
        store: &Arc<InMemoryStore>,
        workflow_name: &str,
        handler_version: Option<&str>,
        path: &[RunStatus],
    ) -> Run {
        let mut labels = HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        let run = store
            .create_run(NewRun {
                workflow_name: workflow_name.to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({"key": "value", "nested": {"n": 1}}),
                max_retries: 2,
                handler_version: handler_version.map(str::to_string),
                labels,
                scheduled_at: None,
                created_by: None,
                idempotency_key: Some(format!("key-{}", Uuid::now_v7())),
                max_cost_usd: Some(Decimal::new(250, 2)),
            })
            .await
            .unwrap()
            .into_run();

        for status in path {
            store.update_run_status(run.id, *status).await.unwrap();
        }
        store.get_run(run.id).await.unwrap().unwrap()
    }

    async fn send_replay(state: AppState, auth_header: String, id: Uuid) -> Response<Body> {
        let app = Router::new()
            .route("/{id}/replay", post(replay_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{id}/replay"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        app.oneshot(req).await.unwrap()
    }

    async fn new_run_id(resp: Response<Body>) -> Uuid {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        from_value(json_val["data"]["id"].clone()).unwrap()
    }

    async fn replay_status(path: &[RunStatus]) -> HttpStatusCode {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run_in(&store, WORKFLOW, Some("2.0.0"), path).await;
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        send_replay(state, auth_header, run.id).await.status()
    }

    #[tokio::test]
    async fn replay_completed_run_succeeds() {
        let store = Arc::new(InMemoryStore::new());
        let original = create_run_in(
            &store,
            WORKFLOW,
            Some("2.0.0"),
            &[RunStatus::Running, RunStatus::Completed],
        )
        .await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let resp = send_replay(state, auth_header, original.id).await;
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let new_id = new_run_id(resp).await;
        assert_ne!(new_id, original.id);

        let new_run = store.get_run(new_id).await.unwrap().unwrap();
        assert_eq!(new_run.status.state, RunStatus::Pending);
        assert_eq!(
            new_run.trigger,
            TriggerKind::Replay {
                original_run_id: original.id,
            }
        );
        assert_eq!(new_run.workflow_name, original.workflow_name);
        assert_eq!(new_run.payload, original.payload);
        assert_eq!(new_run.labels, original.labels);
        assert_eq!(new_run.max_cost_usd, original.max_cost_usd);
        assert_eq!(new_run.max_retries, original.max_retries);
        assert_eq!(new_run.idempotency_key, None);
    }

    #[tokio::test]
    async fn replay_failed_run_succeeds() {
        let status = replay_status(&[RunStatus::Running, RunStatus::Failed]).await;
        assert_eq!(status, HttpStatusCode::CREATED);
    }

    #[tokio::test]
    async fn replay_warning_run_succeeds() {
        let status = replay_status(&[RunStatus::Running, RunStatus::Warning]).await;
        assert_eq!(status, HttpStatusCode::CREATED);
    }

    #[tokio::test]
    async fn replay_cancelled_run_succeeds() {
        let status = replay_status(&[RunStatus::Cancelled]).await;
        assert_eq!(status, HttpStatusCode::CREATED);
    }

    #[tokio::test]
    async fn replay_pending_run_returns_400() {
        let status = replay_status(&[]).await;
        assert_eq!(status, HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn replay_running_run_returns_400() {
        let status = replay_status(&[RunStatus::Running]).await;
        assert_eq!(status, HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn replay_retrying_run_returns_400() {
        let status = replay_status(&[RunStatus::Running, RunStatus::Retrying]).await;
        assert_eq!(status, HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn replay_awaiting_approval_run_returns_400() {
        let status = replay_status(&[RunStatus::Running, RunStatus::AwaitingApproval]).await;
        assert_eq!(status, HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn replay_sleeping_run_returns_400() {
        let status = replay_status(&[RunStatus::Running, RunStatus::Sleeping]).await;
        assert_eq!(status, HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn replay_in_flight_run_creates_no_run() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run_in(&store, WORKFLOW, Some("2.0.0"), &[RunStatus::Running]).await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let resp = send_replay(state, auth_header, run.id).await;
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let msg = json_val["error"]["message"].as_str().unwrap();
        assert!(msg.contains("Running"));

        let runs = store.list_runs(RunFilter::default(), 1, 50).await.unwrap();
        assert_eq!(runs.total, 1);
    }

    #[tokio::test]
    async fn replay_uses_current_handler_version_not_original() {
        let store = Arc::new(InMemoryStore::new());
        let original = create_run_in(
            &store,
            WORKFLOW,
            Some("1.0.0"),
            &[RunStatus::Running, RunStatus::Completed],
        )
        .await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let resp = send_replay(state, auth_header, original.id).await;
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let new_id = new_run_id(resp).await;
        let new_run = store.get_run(new_id).await.unwrap().unwrap();
        assert_eq!(new_run.handler_version, Some("2.0.0".to_string()));
    }

    #[tokio::test]
    async fn replay_unregistered_workflow_returns_404() {
        let store = Arc::new(InMemoryStore::new());
        let original = create_run_in(
            &store,
            "removed-wf",
            Some("1.0.0"),
            &[RunStatus::Running, RunStatus::Completed],
        )
        .await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let resp = send_replay(state, auth_header, original.id).await;
        assert_eq!(resp.status(), HttpStatusCode::NOT_FOUND);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "WORKFLOW_NOT_FOUND");
    }

    #[tokio::test]
    async fn replay_nonexistent_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let resp = send_replay(state, auth_header, Uuid::now_v7()).await;
        assert_eq!(resp.status(), HttpStatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn replay_as_non_admin_returns_403() {
        let store = Arc::new(InMemoryStore::new());
        let original = create_run_in(
            &store,
            WORKFLOW,
            Some("2.0.0"),
            &[RunStatus::Running, RunStatus::Completed],
        )
        .await;

        let state = test_state(store);
        let token =
            AccessToken::for_user(Uuid::now_v7(), "viewer", false, &state.jwt_config).unwrap();
        let resp = send_replay(state, format!("Bearer {}", token.0), original.id).await;
        assert_eq!(resp.status(), HttpStatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn replay_does_not_mutate_the_original_run() {
        let store = Arc::new(InMemoryStore::new());
        let original = create_run_in(
            &store,
            WORKFLOW,
            Some("1.0.0"),
            &[RunStatus::Running, RunStatus::Failed],
        )
        .await;

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let resp = send_replay(state, auth_header, original.id).await;
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let after = store.get_run(original.id).await.unwrap().unwrap();
        assert_eq!(after.status.state, RunStatus::Failed);
        assert_eq!(after.trigger, TriggerKind::Manual);
        assert_eq!(after.handler_version, Some("1.0.0".to_string()));
        assert_eq!(after.payload, original.payload);
        assert_eq!(after.idempotency_key, original.idempotency_key);
        assert_eq!(after.updated_at, original.updated_at);
    }

    #[tokio::test]
    async fn replay_attributes_the_new_run_to_the_caller() {
        let store = Arc::new(InMemoryStore::new());
        let original_author = store
            .create_user(NewUser {
                email: "alice@example.com".to_string(),
                username: "alice".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(true),
            })
            .await
            .unwrap();
        let replaying_user = store
            .create_user(NewUser {
                email: "bob@example.com".to_string(),
                username: "bob".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(true),
            })
            .await
            .unwrap();

        let run = store
            .create_run(NewRun {
                workflow_name: WORKFLOW.to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: Some(RunActor::User {
                    user_id: original_author.id,
                }),
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
            .update_run_status(run.id, RunStatus::Completed)
            .await
            .unwrap();

        let state = test_state(store);
        let token =
            AccessToken::for_user(replaying_user.id, "bob", true, &state.jwt_config).unwrap();
        let resp = send_replay(state, format!("Bearer {}", token.0), run.id).await;
        assert_eq!(resp.status(), HttpStatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["created_by"]["kind"], "user");
        assert_eq!(
            json_val["data"]["created_by"]["id"],
            replaying_user.id.to_string()
        );
        assert_eq!(json_val["data"]["created_by"]["label"], "bob");
    }
}
