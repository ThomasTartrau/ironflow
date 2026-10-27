//! `GET /api/v1/stats/agent-roles` -- Cost and executions per agent role.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use chrono::{Duration, Utc};
use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::ApiKeyScope;

use crate::authorization::AuthorizeExt;
use crate::entities::{AgentRoleStatsEntryResponse, AgentRoleStatsQuery, AgentRoleStatsResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get the cost, executions and failures of every agent role over a period.
///
/// Aggregates the agent steps created during the period (`24h`, `7d`, `30d`
/// or `90d`, default `7d`) by the `AgentRole` they were created from, all
/// workflows combined. Each step record counts as one execution, retried
/// attempts included. Agent steps without a role are left out.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/stats/agent-roles",
        tags = ["stats"],
        params(AgentRoleStatsQuery),
        responses(
            (status = 200, description = "Statistics per agent role", body = AgentRoleStatsResponse),
            (status = 400, description = "Invalid period"),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_agent_role_stats(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<AgentRoleStatsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth.require(ApiKeyScope::StatsRead, false)?;
    let period = params.period.unwrap_or_default();
    let created_after = Utc::now() - Duration::hours(period.hours());

    let roles = state.store.get_agent_role_stats(created_after).await?;

    Ok(ok(AgentRoleStatsResponse {
        period,
        roles: roles
            .into_iter()
            .map(AgentRoleStatsEntryResponse::from)
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewStep, StepKind, StepStatus, StepUpdate, step_trace_id};
    use ironflow_store::store::RunStore;
    use rust_decimal::Decimal;
    use serde_json::{Value as JsonValue, from_slice, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::routes::test_helpers::{
        create_named_run, create_user_auth_header, state_with_engine,
    };

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
        state_with_engine(store, engine, false)
    }

    async fn create_run(store: &InMemoryStore, workflow: &str) -> Uuid {
        create_named_run(store, workflow).await.id
    }

    /// Create an agent step of `role` and run it to `status`.
    async fn role_step(
        store: &InMemoryStore,
        run_id: Uuid,
        role: &str,
        status: StepStatus,
        cost_usd: Decimal,
        duration_ms: u64,
    ) {
        let step = store
            .create_step(NewStep {
                run_id,
                trace_id: step_trace_id(run_id, "review", 0),
                name: "review".to_string(),
                kind: StepKind::Agent,
                position: 0,
                input: Some(json!({"type": "agent", "prompt": "p", "role": role})),
                is_error_handler: false,
            })
            .await
            .expect("create step");
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("start step");
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(status),
                    cost_usd: Some(cost_usd),
                    duration_ms: Some(duration_ms),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("finish step");
    }

    async fn fetch(store: Arc<InMemoryStore>, uri: &str) -> (StatusCode, JsonValue) {
        let state = test_state(store);
        let auth_header = create_user_auth_header(&state, "testuser", false).await;
        let app = Router::new()
            .route("/", get(get_agent_role_stats))
            .with_state(state);

        let req = Request::builder()
            .uri(uri)
            .header("authorization", auth_header)
            .body(Body::empty())
            .expect("request");
        let resp = app.oneshot(req).await.expect("response");
        let status = resp.status();
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let json_val: JsonValue = if body.is_empty() {
            JsonValue::Null
        } else {
            from_slice(&body).unwrap_or(JsonValue::Null)
        };
        (status, json_val)
    }

    #[tokio::test]
    async fn stats_by_role_sum_cost_and_count_executions() {
        let store = Arc::new(InMemoryStore::new());
        let deploy = create_run(&store, "deploy").await;
        let release = create_run(&store, "release").await;
        role_step(
            &store,
            deploy,
            "Reviewer",
            StepStatus::Completed,
            Decimal::new(150, 2),
            1000,
        )
        .await;
        role_step(
            &store,
            release,
            "Reviewer",
            StepStatus::Failed,
            Decimal::new(50, 2),
            500,
        )
        .await;
        role_step(
            &store,
            release,
            "Writer",
            StepStatus::Completed,
            Decimal::new(10, 2),
            100,
        )
        .await;

        let (status, json_val) = fetch(store, "/?period=24h").await;
        assert_eq!(status, StatusCode::OK);

        let data = &json_val["data"];
        assert_eq!(data["period"], "24h");
        let roles = data["roles"].as_array().expect("roles array");
        assert_eq!(roles.len(), 2);
        assert_eq!(roles[0]["agent_role"], "Reviewer");
        assert_eq!(roles[0]["executions"], 2);
        assert_eq!(roles[0]["failed"], 1);
        assert_eq!(roles[0]["total_cost_usd"], 2.0);
        assert_eq!(roles[0]["total_duration_ms"], 1500);
        assert_eq!(roles[1]["agent_role"], "Writer");
        assert_eq!(roles[1]["executions"], 1);
        assert_eq!(roles[1]["failed"], 0);
    }

    #[tokio::test]
    async fn stats_default_to_7d_period() {
        let (status, json_val) = fetch(Arc::new(InMemoryStore::new()), "/").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json_val["data"]["period"], "7d");
        assert!(
            json_val["data"]["roles"]
                .as_array()
                .expect("roles array")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stats_reject_an_unknown_period() {
        let (status, _) = fetch(Arc::new(InMemoryStore::new()), "/?period=1y").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stats_require_auth() {
        let state = test_state(Arc::new(InMemoryStore::new()));
        let app = Router::new()
            .route("/", get(get_agent_role_stats))
            .with_state(state);
        let req = Request::builder()
            .uri("/")
            .body(Body::empty())
            .expect("request");
        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
