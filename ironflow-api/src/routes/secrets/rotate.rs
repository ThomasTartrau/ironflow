//! `POST /api/v1/secrets/rotate` -- Re-encrypt a batch of secrets (admin only).

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde_json::json;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::{EventKind, NewAuditLogEntry, RotationRequest};

use crate::entities::{RotateSecretsRequest, RotateSecretsResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Re-encrypt one batch of secrets towards a key version. Admin only.
///
/// One call processes one batch, so a rotation over a large stock never
/// becomes a single long request. The client loops, passing `last_id` back as
/// `after_id`, until `remaining` reaches zero or `last_id` comes back null.
///
/// The operation is idempotent: secrets already on the target version are
/// skipped, so an interrupted rotation can simply be restarted.
///
/// # Errors
///
/// - 400 if the target version is not in the configured key ring
/// - 401 if the caller is not authenticated
/// - 403 if the caller is not an admin
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/secrets/rotate",
        tags = ["secrets"],
        request_body(
            content = RotateSecretsRequest,
            description = "Target key version, batch size, and resume cursor"
        ),
        responses(
            (status = 200, description = "Batch rotated", body = RotateSecretsResponse),
            (status = 400, description = "Invalid target key version"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn rotate_secrets(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<RotateSecretsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let status = state.store.secret_key_status().await?;
    let to_version = req.to_version.unwrap_or(status.active);

    // Checked here rather than left to the store so the caller gets a 400
    // naming the versions it could have asked for, not an opaque 500.
    if !status.configured.contains(&to_version) {
        return Err(ApiError::BadRequest(format!(
            "key version {to_version} is not configured (available: {})",
            status
                .configured
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let mut request = RotationRequest::new(to_version).with_batch_size(req.effective_batch_size());
    request.after_id = req.after_id;

    let batch = state.store.rotate_secrets(request).await?;

    // A batch that touched nothing is not worth an audit entry: the CLI
    // always issues one final call that finds the stock already rotated.
    if batch.rotated > 0 || batch.failed > 0 {
        state
            .store
            .append_audit_log(NewAuditLogEntry {
                event_type: EventKind::SecretsRotated,
                // Counts only: a secret key or value must never reach the log.
                payload: json!({
                    "to_version": batch.to_version,
                    "rotated": batch.rotated,
                    "failed": batch.failed,
                    "remaining": batch.remaining,
                }),
                run_id: None,
                step_id: None,
                user_id: Some(auth.user_id),
            })
            .await?;
    }

    Ok(ok(RotateSecretsResponse::from(batch)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::crypto::KeyRing;
    use ironflow_store::entities::AuditLogFilter;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::Value;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    struct TestWorkflow;

    impl WorkflowHandler for TestWorkflow {
        fn name(&self) -> &str {
            "test-workflow"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn hex_key(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    /// A state whose ring holds versions 1 and 2, with `active` encrypting.
    fn test_state(active: i32) -> AppState {
        let spec = format!("1:{},2:{}", hex_key(0xaa), hex_key(0xbb));
        let mut in_mem_store = InMemoryStore::new();
        in_mem_store.set_key_ring(KeyRing::from_spec(&spec, Some(active)).unwrap());

        let store: Arc<dyn Store> = Arc::new(in_mem_store);
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(TestWorkflow).unwrap();
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn make_admin_token(state: &AppState) -> String {
        let token =
            AccessToken::for_user(Uuid::now_v7(), "admin", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn make_regular_token(state: &AppState) -> String {
        let token =
            AccessToken::for_user(Uuid::now_v7(), "user", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .route("/rotate", post(rotate_secrets))
            .with_state(state)
    }

    fn rotate_request(auth: Option<&str>, body: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .uri("/rotate")
            .method("POST")
            .header("content-type", "application/json");
        if let Some(auth) = auth {
            builder = builder.header("authorization", auth);
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    async fn json_body(resp: axum::response::Response) -> Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn rotate_requires_authentication() {
        let state = test_state(2);
        let resp = app(state)
            .oneshot(rotate_request(None, "{}"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rotate_is_admin_only() {
        let state = test_state(2);
        let auth = make_regular_token(&state);
        let resp = app(state)
            .oneshot(rotate_request(Some(&auth), "{}"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rotate_moves_secrets_to_the_target_version() {
        let state = test_state(1);
        state.store.set_secret("a", "va").await.unwrap();
        state.store.set_secret("b", "vb").await.unwrap();

        let auth = make_admin_token(&state);
        let store = Arc::clone(&state.store);

        let resp = app(state)
            .oneshot(rotate_request(Some(&auth), r#"{"to_version":2}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = json_body(resp).await;
        assert_eq!(body["data"]["to_version"], 2);
        assert_eq!(body["data"]["rotated"], 2);
        assert_eq!(body["data"]["failed"], 0);
        assert_eq!(body["data"]["remaining"], 0);
        assert!(body["data"]["last_id"].is_string());

        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![2]);
        assert_eq!(store.get_secret("a").await.unwrap().unwrap().value, "va");
    }

    #[tokio::test]
    async fn rotate_defaults_to_the_active_version() {
        let state = test_state(2);
        // Written while the ring is active on 2, so force it onto 1 first by
        // rotating backwards, then let the default bring it back.
        state.store.set_secret("a", "va").await.unwrap();

        let auth = make_admin_token(&state);
        let store = Arc::clone(&state.store);
        let router = app(state);

        let back = router
            .clone()
            .oneshot(rotate_request(Some(&auth), r#"{"to_version":1}"#))
            .await
            .unwrap();
        assert_eq!(back.status(), StatusCode::OK);
        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![1]);

        let resp = router
            .oneshot(rotate_request(Some(&auth), "{}"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = json_body(resp).await;
        assert_eq!(body["data"]["to_version"], 2);
        assert_eq!(body["data"]["rotated"], 1);
        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![2]);
    }

    #[tokio::test]
    async fn rotate_to_unconfigured_version_is_a_bad_request() {
        let state = test_state(2);
        let auth = make_admin_token(&state);

        let resp = app(state)
            .oneshot(rotate_request(Some(&auth), r#"{"to_version":9}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = json_body(resp).await;
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains('9'));
        assert!(message.contains("1, 2"));
    }

    #[tokio::test]
    async fn rotate_rejects_non_positive_version() {
        let state = test_state(2);
        let auth = make_admin_token(&state);

        let resp = app(state)
            .oneshot(rotate_request(Some(&auth), r#"{"to_version":0}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rotate_honours_batch_size_and_cursor() {
        let state = test_state(1);
        for i in 0..3 {
            state.store.set_secret(&format!("k{i}"), "v").await.unwrap();
        }

        let auth = make_admin_token(&state);
        let store = Arc::clone(&state.store);
        let router = app(state);

        let first = router
            .clone()
            .oneshot(rotate_request(
                Some(&auth),
                r#"{"to_version":2,"batch_size":1}"#,
            ))
            .await
            .unwrap();
        let first = json_body(first).await;
        assert_eq!(first["data"]["rotated"], 1);
        assert_eq!(first["data"]["remaining"], 2);

        let cursor = first["data"]["last_id"].as_str().unwrap().to_string();
        let second = router
            .oneshot(rotate_request(
                Some(&auth),
                &format!(r#"{{"to_version":2,"batch_size":10,"after_id":"{cursor}"}}"#),
            ))
            .await
            .unwrap();
        let second = json_body(second).await;
        assert_eq!(second["data"]["rotated"], 2);
        assert_eq!(second["data"]["remaining"], 0);

        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![2]);
    }

    #[tokio::test]
    async fn rotate_writes_an_audit_entry_without_secret_material() {
        let state = test_state(1);
        state
            .store
            .set_secret("workflows/inbox/token", "super-secret-value")
            .await
            .unwrap();

        let auth = make_admin_token(&state);
        let store = Arc::clone(&state.store);

        let resp = app(state)
            .oneshot(rotate_request(Some(&auth), r#"{"to_version":2}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let logs = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();
        let entry = logs
            .items
            .iter()
            .find(|e| e.event_type == EventKind::SecretsRotated)
            .expect("a rotation must be audited");

        assert_eq!(entry.payload["to_version"], 2);
        assert_eq!(entry.payload["rotated"], 1);
        assert!(entry.user_id.is_some());

        let payload = entry.payload.to_string();
        assert!(!payload.contains("super-secret-value"));
        assert!(!payload.contains("workflows/inbox/token"));
    }

    #[tokio::test]
    async fn rotate_with_nothing_to_do_writes_no_audit_entry() {
        let state = test_state(2);
        state.store.set_secret("a", "va").await.unwrap();

        let auth = make_admin_token(&state);
        let store = Arc::clone(&state.store);

        let resp = app(state)
            .oneshot(rotate_request(Some(&auth), r#"{"to_version":2}"#))
            .await
            .unwrap();
        let body = json_body(resp).await;
        assert_eq!(body["data"]["rotated"], 0);

        let logs = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();
        assert!(
            !logs
                .items
                .iter()
                .any(|e| e.event_type == EventKind::SecretsRotated)
        );
    }
}
