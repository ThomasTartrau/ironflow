//! `GET /api/v1/audit-logs` -- List audit log entries (admin only).

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::{AuditLogEntry, AuditLogFilter, EventKind};

use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// Query parameters for listing audit log entries.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
pub struct ListAuditLogsQuery {
    /// Filter by event type (e.g. `run_status_changed`).
    pub event_type: Option<EventKind>,
    /// Filter by run ID.
    pub run_id: Option<Uuid>,
    /// Filter entries created at or after this timestamp.
    pub from: Option<DateTime<Utc>>,
    /// Filter entries created at or before this timestamp.
    pub to: Option<DateTime<Utc>>,
    /// Page number (1-based, default: 1).
    pub page: Option<u32>,
    /// Items per page (default: 50, max: 100).
    pub per_page: Option<u32>,
}

/// List audit log entries with optional filtering and pagination.
///
/// Admin-only. Returns a paginated list of persisted domain events
/// for compliance review and post-mortem debugging.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/audit-logs",
        tags = ["audit"],
        params(ListAuditLogsQuery),
        responses(
            (status = 200, description = "List of audit log entries with pagination", body = Vec<AuditLogEntry>),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden - admin only")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_audit_logs(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ListAuditLogsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(50).min(100);

    let filter = AuditLogFilter {
        event_type: params.event_type,
        run_id: params.run_id,
        from: params.from,
        to: params.to,
    };

    let page_result = state.store.list_audit_logs(filter, page, per_page).await?;

    Ok(ok_paged(
        page_result.items,
        page,
        per_page,
        page_result.total,
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use serde_json::{Value as JsonValue, from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{EventKind, NewAuditLogEntry};
    use ironflow_store::memory::InMemoryStore;

    use super::*;

    fn new_test_entry(event_type: EventKind, run_id: Option<Uuid>) -> NewAuditLogEntry {
        NewAuditLogEntry {
            event_type,
            payload: json!({}),
            run_id,
            step_id: None,
            user_id: None,
        }
    }

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
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

    fn make_admin_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "admin", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn make_user_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "user", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn empty_list() {
        let state = test_state();
        let auth_header = make_admin_auth_header(&state);
        let app = Router::new()
            .route("/", get(list_audit_logs))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 0);
        assert_eq!(json_val["meta"]["total"], 0);
    }

    #[tokio::test]
    async fn non_admin_gets_403() {
        let state = test_state();
        let auth_header = make_user_auth_header(&state);
        let app = Router::new()
            .route("/", get(list_audit_logs))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn returns_entries_with_pagination() {
        let state = test_state();
        let auth_header = make_admin_auth_header(&state);

        let kinds = [
            EventKind::RunCreated,
            EventKind::RunFailed,
            EventKind::StepCompleted,
            EventKind::StepFailed,
            EventKind::UserSignedIn,
        ];
        for kind in kinds {
            state
                .store
                .append_audit_log(new_test_entry(kind, None))
                .await
                .unwrap();
        }

        let app = Router::new()
            .route("/", get(list_audit_logs))
            .with_state(state);

        let req = Request::builder()
            .uri("/?page=1&per_page=2")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 2);
        assert_eq!(json_val["meta"]["total"], 5);
        assert_eq!(json_val["meta"]["page"], 1);
        assert_eq!(json_val["meta"]["per_page"], 2);
    }

    #[tokio::test]
    async fn filters_by_event_type() {
        let state = test_state();
        let auth_header = make_admin_auth_header(&state);

        state
            .store
            .append_audit_log(new_test_entry(EventKind::RunCreated, None))
            .await
            .unwrap();
        state
            .store
            .append_audit_log(new_test_entry(EventKind::RunFailed, None))
            .await
            .unwrap();

        let app = Router::new()
            .route("/", get(list_audit_logs))
            .with_state(state);

        let req = Request::builder()
            .uri("/?event_type=run_created")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 1);
        assert_eq!(json_val["data"][0]["event_type"], "run_created");
    }

    #[tokio::test]
    async fn filters_by_run_id() {
        let state = test_state();
        let auth_header = make_admin_auth_header(&state);
        let target_run = Uuid::now_v7();

        state
            .store
            .append_audit_log(new_test_entry(EventKind::RunCreated, Some(target_run)))
            .await
            .unwrap();
        state
            .store
            .append_audit_log(new_test_entry(EventKind::RunCreated, Some(Uuid::now_v7())))
            .await
            .unwrap();

        let app = Router::new()
            .route("/", get(list_audit_logs))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/?run_id={target_run}"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn per_page_capped_at_100() {
        let state = test_state();
        let auth_header = make_admin_auth_header(&state);
        let app = Router::new()
            .route("/", get(list_audit_logs))
            .with_state(state);

        let req = Request::builder()
            .uri("/?per_page=500")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["meta"]["per_page"], 100);
    }
}
