//! API key scope checks on every user-facing route, through the real router.
//!
//! Every helper here builds real objects: a real `InMemoryStore`, real users,
//! real API keys stored through `ApiKeyStore` and real JWTs minted by
//! `ironflow_auth`. Requests go through `create_router`, so the extractors run
//! in the same order as in production.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use serde_json::{Value, from_slice, json};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::{
    ApiKeyScope, NewApiKey, NewSchedule, NewUser, RunFilter, SchedulePolicy, ScheduleSource, User,
};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;

use crate::routes::{RouterConfig, create_router};
use crate::state::AppState;

/// Every scope a user-facing route requires.
const ROUTE_SCOPES: [ApiKeyScope; 6] = [
    ApiKeyScope::RunsRead,
    ApiKeyScope::RunsWrite,
    ApiKeyScope::RunsManage,
    ApiKeyScope::WorkflowsRead,
    ApiKeyScope::StatsRead,
    ApiKeyScope::Admin,
];

/// A real router over an in-memory store holding an admin, a member and a
/// throwaway user the `/users/{id}` rows act on.
struct Fixture {
    state: AppState,
    router: Router,
    admin: User,
    member: User,
    target: User,
}

async fn fixture() -> Fixture {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let engine = Arc::new(Engine::new(store.clone(), provider));
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-scopes".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(16);
    let state = AppState::new(
        store.clone(),
        engine,
        jwt_config,
        "test-worker-token".to_string(),
        event_sender,
    );

    let admin = create_user(store.as_ref(), "admin", true).await;
    let member = create_user(store.as_ref(), "member", false).await;
    let target = create_user(store.as_ref(), "target", false).await;
    let router = create_router(
        state.clone(),
        RouterConfig {
            rate_limit_auth: None,
            rate_limit_general: None,
            ..RouterConfig::default()
        },
    );
    Fixture {
        state,
        router,
        admin,
        member,
        target,
    }
}

async fn create_user(store: &dyn Store, username: &str, is_admin: bool) -> User {
    let password_hash = password::hash("password123").expect("hash");
    store
        .create_user(NewUser {
            email: format!("{username}@example.com"),
            username: username.to_string(),
            password_hash,
            is_admin: Some(is_admin),
        })
        .await
        .expect("create user")
}

/// A `Bearer` JWT header for `user`.
fn jwt_header(user: &User, state: &AppState) -> String {
    let token = AccessToken::for_user(user.id, &user.username, user.is_admin, &state.jwt_config)
        .expect("token");
    format!("Bearer {}", token.0)
}

/// A `Bearer` API key header for a key owned by `user` with `scopes`.
///
/// The key is written to the store directly, so a member may hold scopes the
/// `/api-keys` route would refuse them.
async fn api_key_header(user: &User, scopes: Vec<ApiKeyScope>, state: &AppState) -> String {
    let raw_key = format!(
        "irfl_{}rest-of-secret-key",
        &Uuid::now_v7().simple().to_string()[24..]
    );
    let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];
    state
        .store
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

/// One key per route scope, each owned by `user` and holding only that scope.
async fn keys_per_scope(user: &User, state: &AppState) -> HashMap<ApiKeyScope, String> {
    let mut keys = HashMap::new();
    for scope in ROUTE_SCOPES {
        let header = api_key_header(user, vec![scope.clone()], state).await;
        keys.insert(scope, header);
    }
    keys
}

/// A route, the body it needs to reach the handler, and the scope it requires.
struct Row {
    method: &'static str,
    uri: String,
    body: Option<Value>,
    scope: ApiKeyScope,
}

/// A row without a body.
fn row(method: &'static str, uri: impl Into<String>, scope: ApiKeyScope) -> Row {
    Row {
        method,
        uri: uri.into(),
        body: None,
        scope,
    }
}

/// A row sent with a JSON `body`.
fn row_with_body(
    method: &'static str,
    uri: impl Into<String>,
    body: Value,
    scope: ApiKeyScope,
) -> Row {
    Row {
        method,
        uri: uri.into(),
        body: Some(body),
        scope,
    }
}

/// A valid `POST /runs` body, for a workflow that is not registered.
fn create_run_body() -> Value {
    json!({"workflow": "test-workflow", "payload": {"env": "prod"}})
}

/// A valid `POST /schedules` body, for a workflow that is not registered.
fn create_schedule_body() -> Value {
    json!({"workflow_name": "test-workflow", "cron_expression": "0 0 * * * *"})
}

/// A valid `POST /approval-delegations` body delegating to `to_user_id`.
fn create_delegation_body(to_user_id: Uuid) -> Value {
    json!({"to_user_id": to_user_id, "valid_until": "2099-01-01T00:00:00Z"})
}

/// A valid `POST /users` body.
fn create_user_body() -> Value {
    json!({
        "email": "scope-new@example.com",
        "username": "scopenew",
        "password": "Plinth-Quarry-Ember-42",
        "is_admin": false
    })
}

/// Every user-facing scoped route except `GET /templates/registry`, with a
/// valid body where the route has a JSON extractor, so the request always
/// reaches the handler. The registry route clones a git repository on success,
/// so a dedicated deny test covers it instead.
fn rows(run_id: Uuid, step_id: Uuid, user_id: Uuid, schedule_id: Uuid) -> Vec<Row> {
    let run = format!("/api/v1/runs/{run_id}");
    let step = format!("{run}/steps/{step_id}");
    let user = format!("/api/v1/users/{user_id}");
    let schedule = format!("/api/v1/schedules/{schedule_id}");
    let mut rows = vec![
        // runs_read
        row("GET", "/api/v1/runs", ApiKeyScope::RunsRead),
        row("GET", run.clone(), ApiKeyScope::RunsRead),
        row("GET", format!("{run}/logs"), ApiKeyScope::RunsRead),
        row("GET", format!("{run}/events"), ApiKeyScope::RunsRead),
        row("GET", format!("{step}/artifacts/x"), ApiKeyScope::RunsRead),
        row("GET", "/api/v1/events", ApiKeyScope::RunsRead),
        // runs_write
        row_with_body(
            "POST",
            "/api/v1/runs",
            create_run_body(),
            ApiKeyScope::RunsWrite,
        ),
        row_with_body(
            "POST",
            "/api/v1/schedules",
            create_schedule_body(),
            ApiKeyScope::RunsWrite,
        ),
        row("DELETE", schedule.clone(), ApiKeyScope::RunsWrite),
        row("POST", format!("{schedule}/pause"), ApiKeyScope::RunsWrite),
        row("POST", format!("{schedule}/resume"), ApiKeyScope::RunsWrite),
        row(
            "POST",
            format!("{schedule}/trigger"),
            ApiKeyScope::RunsWrite,
        ),
        // runs_manage
        row("POST", format!("{run}/approve"), ApiKeyScope::RunsManage),
        row("POST", format!("{run}/reject"), ApiKeyScope::RunsManage),
        row_with_body(
            "POST",
            format!("{step}/input"),
            json!({"answer": "ok"}),
            ApiKeyScope::RunsManage,
        ),
        row("POST", format!("{step}/reject"), ApiKeyScope::RunsManage),
        row("POST", format!("{run}/cancel"), ApiKeyScope::RunsManage),
        row("POST", format!("{run}/retry"), ApiKeyScope::RunsManage),
        row("POST", format!("{run}/replay"), ApiKeyScope::RunsManage),
        row("POST", format!("{run}/pause"), ApiKeyScope::RunsManage),
        row("POST", format!("{run}/resume"), ApiKeyScope::RunsManage),
        row(
            "GET",
            "/api/v1/approval-delegations",
            ApiKeyScope::RunsManage,
        ),
        row_with_body(
            "POST",
            "/api/v1/approval-delegations",
            create_delegation_body(user_id),
            ApiKeyScope::RunsManage,
        ),
        row(
            "DELETE",
            format!("/api/v1/approval-delegations/{}", Uuid::now_v7()),
            ApiKeyScope::RunsManage,
        ),
        // workflows_read
        row("GET", "/api/v1/workflows", ApiKeyScope::WorkflowsRead),
        row("GET", "/api/v1/schedules", ApiKeyScope::WorkflowsRead),
        row("GET", schedule.clone(), ApiKeyScope::WorkflowsRead),
        row("GET", "/api/v1/workflows/x", ApiKeyScope::WorkflowsRead),
        row_with_body(
            "POST",
            "/api/v1/workflows/x/plan",
            json!({}),
            ApiKeyScope::WorkflowsRead,
        ),
        // stats_read
        row("GET", "/api/v1/stats", ApiKeyScope::StatsRead),
        row("GET", "/api/v1/stats/history", ApiKeyScope::StatsRead),
        // admin
        row("GET", "/api/v1/secrets", ApiKeyScope::Admin),
        row_with_body(
            "POST",
            "/api/v1/secrets",
            json!({"key": "scope/test", "value": "v"}),
            ApiKeyScope::Admin,
        ),
        row_with_body(
            "PUT",
            "/api/v1/secrets/foo",
            json!({"value": "v"}),
            ApiKeyScope::Admin,
        ),
        row("DELETE", "/api/v1/secrets/foo", ApiKeyScope::Admin),
        row_with_body(
            "POST",
            "/api/v1/secrets/rotate",
            json!({}),
            ApiKeyScope::Admin,
        ),
        row("GET", "/api/v1/secrets/key-versions", ApiKeyScope::Admin),
        row("GET", "/api/v1/users", ApiKeyScope::Admin),
        row_with_body(
            "POST",
            "/api/v1/users",
            create_user_body(),
            ApiKeyScope::Admin,
        ),
        row_with_body(
            "PATCH",
            format!("{user}/role"),
            json!({"is_admin": true}),
            ApiKeyScope::Admin,
        ),
        row("GET", format!("{user}/groups"), ApiKeyScope::Admin),
        row_with_body(
            "PUT",
            format!("{user}/groups"),
            json!({"groups": []}),
            ApiKeyScope::Admin,
        ),
        row("DELETE", user.clone(), ApiKeyScope::Admin),
        row("GET", "/api/v1/audit-logs", ApiKeyScope::Admin),
        row("POST", "/api/v1/workflows/x/pause", ApiKeyScope::Admin),
        row("POST", "/api/v1/workflows/x/resume", ApiKeyScope::Admin),
    ];
    // The route only exists in builds with the `prometheus` feature.
    if cfg!(feature = "prometheus") {
        rows.push(row("GET", "/api/v1/metrics", ApiKeyScope::StatsRead));
    }
    rows
}

/// The rows of [`rows`] for a fixture, with a fresh run and step id.
fn fixture_rows(fx: &Fixture) -> Vec<Row> {
    rows(Uuid::now_v7(), Uuid::now_v7(), fx.target.id, Uuid::now_v7())
}

/// Send `method` on `uri` with `auth`, and `body` as JSON when set.
///
/// The response body is left unread: an SSE stream that was let through
/// never ends.
async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    auth: &str,
    body: Option<Value>,
) -> Response {
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
    router.clone().oneshot(req).await.expect("request")
}

/// The `error.code` of a JSON error response.
async fn error_code(resp: Response) -> Value {
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let body: Value = from_slice(&bytes).expect("json");
    body["error"]["code"].clone()
}

/// Assert that `resp` is a 403 carrying `code`.
async fn assert_forbidden_with(resp: Response, code: &str, method: &str, uri: &str) {
    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {uri}");
    assert_eq!(error_code(resp).await, code, "{method} {uri}");
}

/// A scope the row does not accept, and that does not imply it.
fn wrong_scope(required: &ApiKeyScope) -> ApiKeyScope {
    match required {
        ApiKeyScope::StatsRead => ApiKeyScope::RunsRead,
        _ => ApiKeyScope::StatsRead,
    }
}

#[tokio::test]
async fn admin_key_without_the_scope_gets_insufficient_scope_on_every_route() {
    let fx = fixture().await;
    let keys = keys_per_scope(&fx.admin, &fx.state).await;

    for r in fixture_rows(&fx) {
        let auth = &keys[&wrong_scope(&r.scope)];
        let resp = send(&fx.router, r.method, &r.uri, auth, r.body).await;
        assert_forbidden_with(resp, "INSUFFICIENT_SCOPE", r.method, &r.uri).await;
    }
}

#[tokio::test]
async fn admin_key_with_the_scope_passes_every_route() {
    let fx = fixture().await;
    let keys = keys_per_scope(&fx.admin, &fx.state).await;

    for r in fixture_rows(&fx) {
        let auth = &keys[&r.scope];
        let resp = send(&fx.router, r.method, &r.uri, auth, r.body).await;
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{} {} with scope {}",
            r.method,
            r.uri,
            r.scope
        );
    }
}

#[tokio::test]
async fn admin_scope_key_passes_every_route() {
    let fx = fixture().await;
    let auth = api_key_header(&fx.admin, vec![ApiKeyScope::Admin], &fx.state).await;

    for r in fixture_rows(&fx) {
        let resp = send(&fx.router, r.method, &r.uri, &auth, r.body).await;
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{} {}",
            r.method,
            r.uri
        );
    }
}

#[tokio::test]
async fn runs_read_key_cannot_delete_secret_list_users_or_cancel_run() {
    let fx = fixture().await;
    let auth = api_key_header(&fx.admin, vec![ApiKeyScope::RunsRead], &fx.state).await;

    for (method, uri) in [
        ("DELETE", "/api/v1/secrets/foo"),
        ("GET", "/api/v1/users"),
        (
            "POST",
            "/api/v1/runs/00000000-0000-0000-0000-000000000000/cancel",
        ),
    ] {
        let resp = send(&fx.router, method, uri, &auth, None).await;
        assert_forbidden_with(resp, "INSUFFICIENT_SCOPE", method, uri).await;
    }
}

#[tokio::test]
async fn stats_key_cannot_list_runs_but_reads_stats() {
    let fx = fixture().await;
    let auth = api_key_header(&fx.admin, vec![ApiKeyScope::StatsRead], &fx.state).await;

    let resp = send(&fx.router, "GET", "/api/v1/runs", &auth, None).await;
    assert_forbidden_with(resp, "INSUFFICIENT_SCOPE", "GET", "/api/v1/runs").await;

    let resp = send(&fx.router, "GET", "/api/v1/stats", &auth, None).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn non_admin_owner_with_admin_scope_key_is_forbidden_on_admin_routes() {
    let fx = fixture().await;
    let auth = api_key_header(&fx.member, vec![ApiKeyScope::Admin], &fx.state).await;
    let run_id = Uuid::now_v7();

    for r in fixture_rows(&fx)
        .into_iter()
        .filter(|r| r.scope == ApiKeyScope::Admin)
    {
        let resp = send(&fx.router, r.method, &r.uri, &auth, r.body).await;
        assert_forbidden_with(resp, "FORBIDDEN", r.method, &r.uri).await;
    }

    // Creating and managing runs still needs an admin owner.
    let resp = send(
        &fx.router,
        "POST",
        "/api/v1/runs",
        &auth,
        Some(create_run_body()),
    )
    .await;
    assert_forbidden_with(resp, "FORBIDDEN", "POST", "/api/v1/runs").await;

    let cancel = format!("/api/v1/runs/{run_id}/cancel");
    let resp = send(&fx.router, "POST", &cancel, &auth, None).await;
    assert_forbidden_with(resp, "FORBIDDEN", "POST", &cancel).await;
}

#[tokio::test]
async fn jwt_sessions_keep_their_scope_free_behaviour() {
    let fx = fixture().await;
    let admin = jwt_header(&fx.admin, &fx.state);
    let member = jwt_header(&fx.member, &fx.state);
    let cancel = format!("/api/v1/runs/{}/cancel", Uuid::now_v7());

    let one_row_per_scope = [
        row("GET", "/api/v1/runs", ApiKeyScope::RunsRead),
        row_with_body(
            "POST",
            "/api/v1/runs",
            create_run_body(),
            ApiKeyScope::RunsWrite,
        ),
        row("POST", cancel.clone(), ApiKeyScope::RunsManage),
        row("GET", "/api/v1/workflows", ApiKeyScope::WorkflowsRead),
        row("GET", "/api/v1/stats", ApiKeyScope::StatsRead),
        row("GET", "/api/v1/users", ApiKeyScope::Admin),
    ];
    for r in one_row_per_scope {
        let resp = send(&fx.router, r.method, &r.uri, &admin, r.body).await;
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{} {}",
            r.method,
            r.uri
        );
    }

    for uri in [
        "/api/v1/runs",
        "/api/v1/workflows",
        "/api/v1/stats",
        "/api/v1/schedules",
        "/api/v1/approval-delegations",
    ] {
        let resp = send(&fx.router, "GET", uri, &member, None).await;
        assert_eq!(resp.status(), StatusCode::OK, "GET {uri}");
    }

    let pause = format!("/api/v1/schedules/{}/pause", Uuid::now_v7());
    let resp = send(&fx.router, "POST", &pause, &member, None).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "POST {pause}");

    for (method, uri) in [
        ("GET", "/api/v1/users"),
        ("GET", "/api/v1/secrets"),
        ("POST", cancel.as_str()),
    ] {
        let resp = send(&fx.router, method, uri, &member, None).await;
        assert_forbidden_with(resp, "FORBIDDEN", method, uri).await;
    }
}

#[tokio::test]
async fn stats_read_key_cannot_delete_pause_or_trigger_a_schedule_nor_list_templates() {
    let fx = fixture().await;
    let schedule = fx
        .state
        .store
        .create_schedule(NewSchedule {
            workflow_name: "deploy".to_string(),
            cron_expression: "0 0 * * * *".to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            priority: 0,
            created_by_user_id: Some(fx.admin.id),
            next_trigger_at: None,
            policy: SchedulePolicy::default(),
        })
        .await
        .expect("create schedule");
    let auth = api_key_header(&fx.admin, vec![ApiKeyScope::StatsRead], &fx.state).await;
    let base = format!("/api/v1/schedules/{}", schedule.id);

    for (method, uri) in [
        ("DELETE", base.clone()),
        ("POST", format!("{base}/pause")),
        ("POST", format!("{base}/resume")),
        ("POST", format!("{base}/trigger")),
        ("GET", "/api/v1/templates/registry".to_string()),
    ] {
        let resp = send(&fx.router, method, &uri, &auth, None).await;
        assert_forbidden_with(resp, "INSUFFICIENT_SCOPE", method, &uri).await;
    }

    let found = fx
        .state
        .store
        .find_schedule_by_id(schedule.id)
        .await
        .expect("find")
        .expect("schedule still exists");
    assert!(found.disabled_at.is_none());
    let runs = fx
        .state
        .store
        .list_runs(RunFilter::default(), 1, 10)
        .await
        .expect("runs");
    assert_eq!(runs.total, 0);
}

#[tokio::test]
async fn member_key_with_the_scope_reaches_schedule_and_delegation_routes() {
    let fx = fixture().await;
    let keys = keys_per_scope(&fx.member, &fx.state).await;

    for r in fixture_rows(&fx)
        .into_iter()
        .filter(|r| r.uri.contains("/schedules") || r.uri.contains("/approval-delegations"))
    {
        let auth = &keys[&r.scope];
        let resp = send(&fx.router, r.method, &r.uri, auth, r.body).await;
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{} {}",
            r.method,
            r.uri
        );
    }
}
