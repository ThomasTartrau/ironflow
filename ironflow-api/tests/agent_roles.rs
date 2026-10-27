//! Agent role cost through the real API router.
//!
//! An operator reads the cost spent, reserved and left per role, and can only
//! lower the cap declared in code. Everything goes through `create_router`, a
//! real `InMemoryStore`, real JWTs and real API keys.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt;
use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::agent_role::CostPeriod;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::agent_role_store::AgentRoleStore;
use ironflow_store::api_key_store::ApiKeyStore;
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::entities::{
    ApiKeyScope, AuditLogFilter, EventKind, NewApiKey, NewUser, RoleCostReservationRequest,
    RoleCostSettlement, User,
};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::user_store::UserStore;
use rust_decimal::Decimal;
use serde_json::{Value, from_slice, json};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;

/// A real router over an in-memory store holding an admin and a member.
struct App {
    store: Arc<InMemoryStore>,
    router: Router,
    state: AppState,
    admin: User,
    member: User,
}

async fn app() -> App {
    let store = Arc::new(InMemoryStore::new());
    let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-agent-roles".to_string(),
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
    // The limiter keys on the peer address, which `oneshot` does not provide.
    let config = RouterConfig {
        rate_limit_auth: None,
        rate_limit_general: None,
        ..RouterConfig::default()
    };
    let router = create_router(state.clone(), config);
    let admin = create_user(&store, "admin", true).await;
    let member = create_user(&store, "member", false).await;
    App {
        store,
        router,
        state,
        admin,
        member,
    }
}

async fn create_user(store: &InMemoryStore, username: &str, is_admin: bool) -> User {
    store
        .create_user(NewUser {
            email: format!("{username}@test.com"),
            username: username.to_string(),
            password_hash: "argon2hash".to_string(),
            is_admin: Some(is_admin),
        })
        .await
        .expect("create user")
}

fn jwt(app: &App, user: &User) -> String {
    let config = &app.state.jwt_config;
    let token =
        AccessToken::for_user(user.id, &user.username, user.is_admin, config).expect("token");
    format!("Bearer {}", token.0)
}

/// A `Bearer` API key owned by `user` with `scopes`.
async fn api_key(app: &App, user: &User, scopes: Vec<ApiKeyScope>) -> String {
    let raw_key = format!(
        "irfl_{}rest-of-secret-key",
        &Uuid::now_v7().simple().to_string()[24..]
    );
    let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];
    app.store
        .create_api_key(NewApiKey {
            user_id: user.id,
            name: format!("key-{}", Uuid::now_v7()),
            key_hash: password::hash(&raw_key).expect("hash"),
            key_prefix: prefix.to_string(),
            scopes,
            expires_at: None,
            rate_limit_override: None,
        })
        .await
        .expect("create api key");
    format!("Bearer {raw_key}")
}

async fn call(
    app: &App,
    method: &str,
    uri: &str,
    auth: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("authorization", auth);
    let req = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("build request");
    let resp = app.router.clone().oneshot(req).await.expect("request");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        from_slice(&bytes).expect("json")
    };
    (status, value)
}

/// Settle a step costing `spent` for `role` (code cap `cap`) and keep one
/// holding `reserved` running.
async fn seed(app: &App, role: &str, cap: Option<Decimal>, spent: Decimal, reserved: Decimal) {
    let period_start = CostPeriod::Month.start(Utc::now());
    let request = |budget: Decimal| RoleCostReservationRequest {
        reservation_id: Uuid::now_v7(),
        role: role.to_string(),
        code_cap_usd: cap,
        period: "month".to_string(),
        period_start,
        budget_usd: Some(budget),
        ttl_secs: 600,
    };
    let settled = request(spent);
    let settled_id = settled.reservation_id;
    app.store
        .reserve_agent_role_cost(settled)
        .await
        .expect("reserve");
    app.store
        .settle_agent_role_cost(RoleCostSettlement {
            reservation_id: settled_id,
            period: "month".to_string(),
            period_start,
            cost_usd: spent,
        })
        .await
        .expect("settle");
    app.store
        .reserve_agent_role_cost(request(reserved))
        .await
        .expect("reserve running step");
}

const REVIEWER_CAP: &str = "/api/v1/agent-roles/reviewer/cost-cap";

#[tokio::test]
async fn list_and_get_show_spent_reserved_and_remaining() {
    let app = app().await;
    seed(
        &app,
        "reviewer",
        Some(Decimal::TEN),
        Decimal::new(25, 1),
        Decimal::ONE,
    )
    .await;
    seed(&app, "triage", None, Decimal::ONE, Decimal::ZERO).await;
    let auth = jwt(&app, &app.member);

    let (status, body) = call(&app, "GET", "/api/v1/agent-roles", &auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["meta"]["total"], 2);
    let names: Vec<_> = body["data"]
        .as_array()
        .expect("array")
        .iter()
        .map(|r| r["name"].as_str().expect("name").to_string())
        .collect();
    assert_eq!(names, ["reviewer", "triage"]);
    assert!(body["data"][1]["cap_usd"].is_null());
    assert!(body["data"][1]["remaining_usd"].is_null());
    assert_eq!(body["data"][1]["spent_usd"], 1.0);

    let (status, body) = call(&app, "GET", "/api/v1/agent-roles/reviewer", &auth, None).await;
    assert_eq!(status, StatusCode::OK);
    let role = &body["data"];
    assert_eq!(role["name"], "reviewer");
    assert_eq!(role["period"], "month");
    assert_eq!(role["spent_usd"], 2.5);
    assert_eq!(role["reserved_usd"], 1.0);
    assert_eq!(role["cap_usd"], 10.0);
    assert_eq!(role["code_cap_usd"], 10.0);
    assert!(role["operator_cap_usd"].is_null());
    assert_eq!(role["remaining_usd"], 6.5);
    let resets_at = role["resets_at"].as_str().expect("resets_at");
    assert!(resets_at.contains("T00:00:00"), "{resets_at}");
    assert!(role.get("id").is_none(), "the DTO exposes no internal id");
}

#[tokio::test]
async fn admin_lowers_the_cap_and_it_is_audited() {
    let app = app().await;
    seed(
        &app,
        "reviewer",
        Some(Decimal::TEN),
        Decimal::TWO,
        Decimal::ZERO,
    )
    .await;
    let admin = jwt(&app, &app.admin);

    let body = json!({"operator_cap_usd": 3.0});
    let (status, resp) = call(&app, "PUT", REVIEWER_CAP, &admin, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["data"]["operator_cap_usd"], 3.0);
    assert_eq!(resp["data"]["cap_usd"], 3.0);
    assert_eq!(resp["data"]["remaining_usd"], 1.0);

    // An operator cap above the code cap does not raise it.
    let body = json!({"operator_cap_usd": 100.0});
    let (status, resp) = call(&app, "PUT", REVIEWER_CAP, &admin, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["data"]["cap_usd"], 10.0);

    let body = json!({"operator_cap_usd": null});
    let (status, resp) = call(&app, "PUT", REVIEWER_CAP, &admin, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp["data"]["operator_cap_usd"].is_null());
    assert_eq!(resp["data"]["cap_usd"], 10.0);

    let filter = AuditLogFilter {
        event_type: Some(EventKind::AgentRoleCostCapUpdated),
        ..AuditLogFilter::default()
    };
    let entries = app
        .store
        .list_audit_logs(filter, 1, 10)
        .await
        .expect("list audit logs")
        .items;
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().all(|e| e.user_id == Some(app.admin.id)));
    assert!(entries.iter().all(|e| e.payload["role"] == "reviewer"));
}

#[tokio::test]
async fn member_cannot_set_a_cap() {
    let app = app().await;
    seed(
        &app,
        "reviewer",
        Some(Decimal::TEN),
        Decimal::ONE,
        Decimal::ZERO,
    )
    .await;
    let member = jwt(&app, &app.member);

    let body = json!({"operator_cap_usd": 1.0});
    let (status, _) = call(&app, "PUT", REVIEWER_CAP, &member, Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let record = app
        .store
        .get_agent_role("reviewer")
        .await
        .expect("get role")
        .expect("role exists");
    assert_eq!(record.operator_cap_usd, None);
}

#[tokio::test]
async fn negative_cap_is_rejected() {
    let app = app().await;
    seed(
        &app,
        "reviewer",
        Some(Decimal::TEN),
        Decimal::ONE,
        Decimal::ZERO,
    )
    .await;
    let admin = jwt(&app, &app.admin);

    let body = json!({"operator_cap_usd": -0.5});
    let (status, resp) = call(&app, "PUT", REVIEWER_CAP, &admin, Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(resp["error"]["code"], "BAD_REQUEST");
}

/// Send `body` as is and return the status only: a body rejected by the JSON
/// extractor is plain text.
async fn put_raw_status(app: &App, uri: &str, auth: &str, body: &str) -> StatusCode {
    let req = Request::builder()
        .uri(uri)
        .method("PUT")
        .header("authorization", auth)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    app.router
        .clone()
        .oneshot(req)
        .await
        .expect("request")
        .status()
}

#[tokio::test]
async fn a_body_without_the_cap_field_does_not_clear_the_cap() {
    let app = app().await;
    seed(
        &app,
        "reviewer",
        Some(Decimal::TEN),
        Decimal::ONE,
        Decimal::ZERO,
    )
    .await;
    let admin = jwt(&app, &app.admin);
    let body = json!({"operator_cap_usd": 3.0});
    let (status, _) = call(&app, "PUT", REVIEWER_CAP, &admin, Some(body)).await;
    assert_eq!(status, StatusCode::OK);

    for body in [
        "{}",
        r#"{"operator_cap": 5}"#,
        r#"{"operator_cap_usd": null, "x": 1}"#,
    ] {
        let status = put_raw_status(&app, REVIEWER_CAP, &admin, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    let record = app
        .store
        .get_agent_role("reviewer")
        .await
        .expect("get role")
        .expect("role exists");
    assert_eq!(record.operator_cap_usd, Some(Decimal::from(3)));
}

#[tokio::test]
async fn unknown_role_is_not_found() {
    let app = app().await;
    let admin = jwt(&app, &app.admin);

    let (status, resp) = call(&app, "GET", "/api/v1/agent-roles/ghost", &admin, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(resp["error"]["code"], "AGENT_ROLE_NOT_FOUND");

    let body = json!({"operator_cap_usd": 1.0});
    let uri = "/api/v1/agent-roles/ghost/cost-cap";
    let (status, resp) = call(&app, "PUT", uri, &admin, Some(body)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(resp["error"]["code"], "AGENT_ROLE_NOT_FOUND");
}

#[tokio::test]
async fn invalid_role_name_is_rejected() {
    let app = app().await;
    let admin = jwt(&app, &app.admin);

    let (status, _) = call(&app, "GET", "/api/v1/agent-roles/Reviewer", &admin, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let body = json!({"operator_cap_usd": 1.0});
    let uri = "/api/v1/agent-roles/-reviewer/cost-cap";
    let (status, _) = call(&app, "PUT", uri, &admin, Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn api_key_without_stats_read_is_refused() {
    let app = app().await;
    seed(
        &app,
        "reviewer",
        Some(Decimal::TEN),
        Decimal::ONE,
        Decimal::ZERO,
    )
    .await;

    let runs_only = api_key(&app, &app.member, vec![ApiKeyScope::RunsRead]).await;
    let (status, resp) = call(&app, "GET", "/api/v1/agent-roles", &runs_only, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(resp["error"]["code"], "INSUFFICIENT_SCOPE");

    let stats = api_key(&app, &app.member, vec![ApiKeyScope::StatsRead]).await;
    let (status, _) = call(&app, "GET", "/api/v1/agent-roles/reviewer", &stats, None).await;
    assert_eq!(status, StatusCode::OK);
}
