//! `GET /api/v1/schedules/{id}` -- Get a schedule by ID.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;

use crate::entities::ScheduleResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get a schedule by ID.
///
/// # Errors
///
/// - 401 if not authenticated
/// - 404 if the schedule does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/schedules/{id}",
        tags = ["schedules"],
        params(("id" = Uuid, Path, description = "Schedule ID")),
        responses(
            (status = 200, description = "Schedule detail", body = ScheduleResponse),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Schedule not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_schedule(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let schedule = state
        .store
        .find_schedule_by_id(id)
        .await?
        .ok_or(ApiError::ScheduleNotFound(id))?;

    Ok(ok(ScheduleResponse::from(schedule)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewSchedule, NewUser, SchedulePolicy, ScheduleSource};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::{Value, from_slice, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn get_schedule_exposes_default_catchup() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-get-schedule".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config.clone(),
            "test-worker-token".to_string(),
            event_sender,
        );
        let user = store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: password::hash("password123").expect("hash"),
                is_admin: None,
            })
            .await
            .expect("create user");
        let schedule = store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "0 0 * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                priority: 0,
                created_by_user_id: Some(user.id),
                next_trigger_at: None,
                policy: SchedulePolicy::default(),
            })
            .await
            .expect("create schedule");
        let token = AccessToken::for_user(user.id, "testuser", false, &jwt_config).expect("token");
        let app = Router::new()
            .route("/{id}", get(get_schedule))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", schedule.id))
            .header("authorization", format!("Bearer {}", token.0))
            .body(Body::empty())
            .expect("build");
        let resp = app.oneshot(req).await.expect("request");

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let val: Value = from_slice(&body).expect("json");
        let data = &val["data"];
        assert_eq!(data["catchup"], "latest");
        assert_eq!(data["catchup_max"], 10);
        assert_eq!(data["catchup_window_secs"], 86400);
        assert_eq!(data["overlap"], "allow");
        assert_eq!(data["timezone"], "UTC");
    }
}
