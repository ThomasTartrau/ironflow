//! `GET /api/v1/runs/:id` — Get run details with steps.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use tokio::join;
use uuid::Uuid;

use crate::entities::{
    ArtifactResponse, RunDetailResponse, RunResponse, StepAccountResponse, StepResponse,
};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get a run by ID, including all its steps and dependency edges.
///
/// Returns 404 if the run does not exist.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/runs/{id}",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 200, description = "Run details with steps", body = RunDetailResponse),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_run(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let run = state.get_run_or_404(id).await?;

    // Artifacts are fetched for the whole run in one call and grouped by step,
    // exactly like the dependency edges: one query, never one per step.
    let (steps, deps, artifacts) = join!(
        state.store.list_steps(id),
        state.store.list_step_dependencies(id),
        state.store.list_artifacts_for_run(id)
    );
    let steps = steps?;
    let deps = deps?;
    let artifacts = artifacts?;

    let mut deps_map: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for dep in &deps {
        deps_map
            .entry(dep.step_id)
            .or_default()
            .push(dep.depends_on);
    }

    let mut artifacts_map: HashMap<Uuid, Vec<ArtifactResponse>> = HashMap::new();
    for artifact in artifacts {
        artifacts_map
            .entry(artifact.step_id)
            .or_default()
            .push(ArtifactResponse::from(artifact));
    }

    // Accounts are resolved once for the whole run, deduplicated. A deleted
    // account is absent from the map and the step keeps only its `account_id`.
    let account_ids: Vec<Uuid> = steps
        .iter()
        .filter_map(|step| step.account_id)
        .collect::<HashSet<Uuid>>()
        .into_iter()
        .collect();
    let accounts_map: HashMap<Uuid, StepAccountResponse> = if account_ids.is_empty() {
        HashMap::new()
    } else {
        state
            .store
            .list_provider_accounts_by_ids(account_ids)
            .await?
            .into_iter()
            .map(|account| (account.id, StepAccountResponse::from(account)))
            .collect()
    };

    let step_responses: Vec<StepResponse> = steps
        .into_iter()
        .map(|step| {
            let step_deps = deps_map.remove(&step.id).unwrap_or_default();
            let step_artifacts = artifacts_map.remove(&step.id).unwrap_or_default();
            let account = step
                .account_id
                .and_then(|account_id| accounts_map.get(&account_id).cloned());
            let response =
                StepResponse::with_dependencies_and_artifacts(step, step_deps, step_artifacts);
            match account {
                Some(account) => response.with_account(account),
                None => response,
            }
        })
        .collect();

    let active_descendant_count = state.store.list_active_descendants(id).await?.len() as u64;
    let payload = run.payload.clone();
    let response = RunDetailResponse {
        run: RunResponse::from(run),
        steps: step_responses,
        payload,
        active_descendant_count,
    };

    Ok(ok(response))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use chrono::{TimeDelta, Utc};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewProviderAccount, provider_account_secret_key};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{
        NewRun, NewStep, NewUser, RunActor, RunUpdate, StepKind, StepUpdate, TriggerKind,
        step_trace_id,
    };
    use ironflow_store::store::RunStore;
    use serde_json::{Value as JsonValue, from_slice, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::routes::test_helpers::create_user_auth_header;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        Arc::new(InMemoryStore::new());
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
    async fn existing_run() {
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
                concurrency_key: None,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        Arc::new(InMemoryStore::new());
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        );
        let auth_header = create_user_auth_header(&state, "testuser", false).await;
        let app = Router::new().route("/{id}", get(get_run)).with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", run.id))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["run"]["id"], run.id.to_string());
    }

    #[tokio::test]
    async fn not_found() {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", false).await;
        let app = Router::new().route("/{id}", get(get_run)).with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", Uuid::nil()))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// A run with one agent step, optionally bound to a freshly created
    /// account. Returns the app, the auth header, the run id and the account id.
    async fn run_with_account_step(
        bind: bool,
        delete_account: bool,
    ) -> (Router, String, Uuid, Uuid) {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", false).await;
        let account_id = Uuid::now_v7();
        state
            .store
            .create_provider_account(NewProviderAccount {
                id: account_id,
                name: "perso".to_string(),
                display_name: "Compte perso".to_string(),
                kind: "claude_subscription".to_string(),
                secret_key: provider_account_secret_key(account_id),
                enabled: true,
                priority: 100,
                tags: Vec::new(),
                max_concurrency: None,
                alert_threshold: 0.8,
                expires_at: Utc::now() + TimeDelta::days(30),
                plan: None,
                created_by: None,
            })
            .await
            .unwrap();
        let run = state
            .store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: None,
                concurrency_key: None,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();
        let step = state
            .store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "ask", 0),
                name: "ask".to_string(),
                kind: StepKind::Agent,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .unwrap();
        if bind {
            state
                .store
                .update_step(
                    step.id,
                    StepUpdate {
                        account_id: Some(account_id),
                        ..StepUpdate::default()
                    },
                )
                .await
                .unwrap();
        }
        if delete_account {
            state
                .store
                .delete_provider_account(account_id)
                .await
                .unwrap();
        }
        let app = Router::new().route("/{id}", get(get_run)).with_state(state);
        (app, auth_header, run.id, account_id)
    }

    async fn first_step_of(app: Router, auth_header: String, run_id: Uuid) -> JsonValue {
        let req = Request::builder()
            .uri(format!("/{run_id}"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        json_val["data"]["steps"][0].clone()
    }

    #[tokio::test]
    async fn step_exposes_its_account() {
        let (app, auth_header, run_id, account_id) = run_with_account_step(true, false).await;
        let step = first_step_of(app, auth_header, run_id).await;
        assert_eq!(step["account_id"], account_id.to_string());
        assert_eq!(step["account"]["name"], "perso");
        assert_eq!(step["account"]["display_name"], "Compte perso");
    }

    #[tokio::test]
    async fn step_without_account_has_null_account() {
        let (app, auth_header, run_id, _) = run_with_account_step(false, false).await;
        let step = first_step_of(app, auth_header, run_id).await;
        assert!(step["account_id"].is_null());
        assert!(step["account"].is_null());
    }

    #[tokio::test]
    async fn step_whose_account_was_deleted_has_null_account() {
        let (app, auth_header, run_id, _) = run_with_account_step(true, true).await;
        let step = first_step_of(app, auth_header, run_id).await;
        assert!(step["account"].is_null());
    }

    #[tokio::test]
    async fn detail_exposes_the_run_author() {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", false).await;
        let user = state
            .store
            .create_user(NewUser {
                email: "alice@example.com".to_string(),
                username: "alice".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();

        let run = state
            .store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Api,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: Default::default(),
                scheduled_at: None,
                created_by: Some(RunActor::User { user_id: user.id }),
                idempotency_key: None,
                concurrency_key: None,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let app = Router::new().route("/{id}", get(get_run)).with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", run.id))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["run"]["created_by"]["kind"], "user");
        assert_eq!(
            json_val["data"]["run"]["created_by"]["id"],
            user.id.to_string()
        );
        assert_eq!(json_val["data"]["run"]["created_by"]["label"], "alice");
    }
    /// Fetch the detail of a run whose handler set `output`, or nothing.
    async fn run_detail_with_output(output: Option<JsonValue>) -> JsonValue {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", false).await;
        let run = state
            .store
            .create_run(NewRun {
                workflow_name: "review".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: None,
                concurrency_key: None,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();
        state
            .store
            .update_run(
                run.id,
                RunUpdate {
                    output,
                    ..RunUpdate::default()
                },
            )
            .await
            .unwrap();

        let app = Router::new().route("/{id}", get(get_run)).with_state(state);
        let req = Request::builder()
            .uri(format!("/{}", run.id))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn detail_exposes_the_run_output() {
        let output = json!({"verdict": "approved", "score": 9, "note": "très bien"});

        let detail = run_detail_with_output(Some(output.clone())).await;

        assert_eq!(detail["data"]["run"]["output"], output);
    }

    #[tokio::test]
    async fn detail_keeps_a_falsy_output() {
        let detail = run_detail_with_output(Some(json!(false))).await;

        assert_eq!(detail["data"]["run"]["output"], json!(false));
    }

    #[tokio::test]
    async fn detail_omits_the_output_key_when_the_handler_set_none() {
        let detail = run_detail_with_output(None).await;

        let run = detail["data"]["run"].as_object().expect("a run object");
        assert!(!run.contains_key("output"), "unexpected output: {run:?}");
    }
}
