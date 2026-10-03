//! `/api/v1/provider-accounts` -- Provider Accounts: AI provider credentials
//! and their usage limits. Admin only.
//!
//! Every handler authorizes first, so a member gets 403 before any lookup.
//! No response carries the credential.

pub mod create;
pub mod delete;
pub mod get;
pub mod kinds;
pub mod list;
pub mod test;
pub mod update;
pub mod usage;

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Router, serve};
    use chrono::{TimeDelta, Utc};
    use http_body_util::BodyExt;
    use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_auth::password;
    use ironflow_core::account::ClaudeSubscriptionKind;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::crypto::MasterKey;
    use ironflow_store::entities::{ApiKeyScope, NewApiKey, NewUser};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::{Value, from_slice, json};
    use tokio::net::TcpListener;
    use tokio::spawn;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::routes::test_helpers::create_user_auth_header;
    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    /// A well-formed `claude setup-token` token.
    pub(crate) const TOKEN: &str = "sk-ant-oat01-provider-account-test-token-0123456789";

    /// How the stub Anthropic API answers `POST /v1/messages`.
    #[derive(Clone, Copy)]
    pub(crate) enum Stub {
        /// 200 with unified 5h and 7d headers.
        Valid,
        /// 429 with a reset header.
        Limited,
        /// 401.
        Unauthorized,
    }

    /// Serve a real local HTTP stub of the Anthropic messages endpoint.
    pub(crate) async fn stub_anthropic(stub: Stub) -> String {
        let reset = (Utc::now() + TimeDelta::hours(2)).timestamp().to_string();
        let handler = move || {
            let reset = reset.clone();
            async move {
                let mut headers = HeaderMap::new();
                let status = match stub {
                    Stub::Valid => {
                        for (name, value) in [
                            (
                                "anthropic-ratelimit-unified-5h-utilization",
                                "0.25".to_string(),
                            ),
                            (
                                "anthropic-ratelimit-unified-5h-status",
                                "allowed".to_string(),
                            ),
                            ("anthropic-ratelimit-unified-5h-reset", reset.clone()),
                            (
                                "anthropic-ratelimit-unified-7d-utilization",
                                "0.5".to_string(),
                            ),
                            (
                                "anthropic-ratelimit-unified-7d-status",
                                "allowed".to_string(),
                            ),
                        ] {
                            headers.insert(name, HeaderValue::from_str(&value).unwrap());
                        }
                        StatusCode::OK
                    }
                    Stub::Limited => {
                        headers.insert(
                            "anthropic-ratelimit-unified-reset",
                            HeaderValue::from_str(&reset).unwrap(),
                        );
                        StatusCode::TOO_MANY_REQUESTS
                    }
                    Stub::Unauthorized => StatusCode::UNAUTHORIZED,
                };
                (status, headers, "{}").into_response()
            }
        };
        let app = Router::new().route("/v1/messages", post(handler));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        spawn(async move {
            serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    /// An `AppState` with encrypted secrets, its Claude kind pointed at `api_base`.
    pub(crate) fn state_with_api(api_base: &str) -> AppState {
        let mut mem_store = InMemoryStore::new();
        mem_store.set_master_key(MasterKey::from_bytes(&[42u8; 32]).unwrap());
        let store: Arc<dyn Store> = Arc::new(mem_store);
        let engine = Arc::new(Engine::new(
            store.clone(),
            Arc::new(ClaudeCodeProvider::new()),
        ));
        let jwt_config = Arc::new(JwtConfig {
            secret: "provider-accounts-test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(16);
        AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
        .with_account_kind(Arc::new(ClaudeSubscriptionKind::with_api_base(api_base)))
    }

    /// A state backed by a stub answering `stub`.
    pub(crate) async fn state_with_stub(stub: Stub) -> AppState {
        state_with_api(&stub_anthropic(stub).await)
    }

    /// A `Bearer` header for a user of this state.
    pub(crate) async fn bearer(state: &AppState, is_admin: bool) -> String {
        let username = if is_admin { "admin" } else { "member" };
        create_user_auth_header(state, username, is_admin).await
    }

    /// A `Bearer` header carrying an admin-owned API key with `scopes`.
    pub(crate) async fn api_key_bearer(state: &AppState, scopes: Vec<ApiKeyScope>) -> String {
        let user = state
            .store
            .create_user(NewUser {
                email: format!("{}@test.com", Uuid::now_v7().simple()),
                username: format!("owner{}", Uuid::now_v7().simple()),
                password_hash: password::hash("pass123").unwrap(),
                is_admin: Some(true),
            })
            .await
            .unwrap();
        let raw_key = format!(
            "irfl_{}rest-of-secret-key",
            &Uuid::now_v7().simple().to_string()[24..]
        );
        let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];
        state
            .store
            .create_api_key(NewApiKey {
                user_id: user.id,
                name: "accounts-key".to_string(),
                key_hash: password::hash(&raw_key).unwrap(),
                key_prefix: prefix.to_string(),
                scopes,
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();
        format!("Bearer {raw_key}")
    }

    /// Send one request through the real router; returns the status and JSON body.
    pub(crate) async fn call(
        state: &AppState,
        method: &str,
        uri: &str,
        auth: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value, String) {
        let app = create_router(state.clone(), RouterConfig::default());
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", auth);
        let body = match body {
            Some(json) => {
                builder = builder.header("content-type", "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.oneshot(builder.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes).to_string();
        let json = from_slice(&bytes).unwrap_or(Value::Null);
        (status, json, text)
    }

    /// Create `name` through the API as an admin; returns the response body.
    pub(crate) async fn create_account(state: &AppState, name: &str) -> Value {
        let auth = bearer(state, true).await;
        let (status, body, _) = call(
            state,
            "POST",
            "/api/v1/provider-accounts",
            &auth,
            Some(json!({
                "name": name,
                "kind": "claude_subscription",
                "token": TOKEN,
                "tags": ["perso"],
                "priority": 10
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }
}
