//! Axum extractors for authenticated callers.
//!
//! Provides three extractors:
//!
//! - [`AuthenticatedUser`] -- JWT only (cookie or `Authorization: Bearer` header)
//! - [`ApiKeyAuth`] -- API key only (`irfl_...` prefix)
//! - [`Authenticated`] -- Dual auth: API key OR JWT

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use ironflow_store::entities::ApiKeyScope;
use ironflow_store::store::Store;
use serde_json::json;
use uuid::Uuid;

use crate::cookies::AUTH_COOKIE_NAME;
use crate::jwt::{AccessToken, JwtConfig};
use crate::password;

// ---------------------------------------------------------------------------
// AuthenticatedUser (JWT only)
// ---------------------------------------------------------------------------

/// An authenticated user extracted from a JWT.
///
/// Use as an Axum handler parameter to enforce JWT authentication.
/// Requires `Arc<JwtConfig>` to be extractable from state via `FromRef`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_auth::extractor::AuthenticatedUser;
///
/// async fn protected(user: AuthenticatedUser) -> String {
///     format!("Hello, {}!", user.username)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    /// The user's unique identifier.
    pub user_id: Uuid,
    /// The user's username.
    pub username: String,
    /// Whether the user is an administrator.
    pub is_admin: bool,
}

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
    Arc<JwtConfig>: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let jwt_config = Arc::<JwtConfig>::from_ref(state);

        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(AUTH_COOKIE_NAME)
            .map(|c| c.value().to_string())
            .or_else(|| {
                parts
                    .headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .map(|t| t.to_string())
            });

        let token = token.ok_or(AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "MISSING_TOKEN",
            message: "No authentication token provided",
        })?;

        let claims = AccessToken::decode(&token, &jwt_config).map_err(|_| AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "INVALID_TOKEN",
            message: "Invalid or expired authentication token",
        })?;

        Ok(AuthenticatedUser {
            user_id: claims.user_id,
            username: claims.username,
            is_admin: claims.is_admin,
        })
    }
}

/// Rejection type when JWT authentication fails.
pub struct AuthRejection {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        (self.status, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// ApiKeyAuth (API key only)
// ---------------------------------------------------------------------------

/// API key prefix used to distinguish API keys from JWT tokens.
pub const API_KEY_PREFIX: &str = "irfl_";

/// Number of hex characters kept after [`API_KEY_PREFIX`] to form the stored prefix.
pub const API_KEY_SUFFIX_LEN: usize = 8;

/// An authenticated caller via API key.
///
/// Use as an Axum handler parameter to enforce API key authentication.
///
/// # Examples
///
/// ```no_run
/// use ironflow_auth::extractor::ApiKeyAuth;
///
/// async fn protected(key: ApiKeyAuth) -> String {
///     format!("Key {} (user {})", key.key_name, key.user_id)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ApiKeyAuth {
    /// The API key ID.
    pub key_id: Uuid,
    /// The owner user ID.
    pub user_id: Uuid,
    /// The API key name.
    pub key_name: String,
    /// Scopes granted to this key.
    pub scopes: Vec<ApiKeyScope>,
    /// Whether the key owner is an admin (checked at request time).
    pub owner_is_admin: bool,
}

impl ApiKeyAuth {
    /// Check if the API key has a specific scope.
    pub fn has_scope(&self, required: &ApiKeyScope) -> bool {
        ApiKeyScope::has_permission(&self.scopes, required)
    }
}

/// Rejection type when API key authentication fails.
pub struct ApiKeyRejection {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for ApiKeyRejection {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        (self.status, Json(body)).into_response()
    }
}

impl<S> FromRequestParts<S> for ApiKeyAuth
where
    S: Send + Sync,
    Arc<dyn Store>: FromRef<S>,
{
    type Rejection = ApiKeyRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let store = Arc::<dyn Store>::from_ref(state);

        let token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "MISSING_TOKEN",
                message: "No authentication token provided",
            })?;

        if !token.starts_with(API_KEY_PREFIX) {
            return Err(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "INVALID_TOKEN",
                message: "Expected API key (irfl_...) in Authorization header",
            });
        }

        let suffix_len = (token.len() - API_KEY_PREFIX.len()).min(API_KEY_SUFFIX_LEN);
        let prefix = &token[..API_KEY_PREFIX.len() + suffix_len];

        let api_key = store
            .find_api_key_by_prefix(prefix)
            .await
            .map_err(|_| ApiKeyRejection {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "INTERNAL_ERROR",
                message: "Failed to look up API key",
            })?
            .ok_or(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "INVALID_TOKEN",
                message: "Invalid API key",
            })?;

        if !api_key.is_active {
            return Err(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "KEY_DISABLED",
                message: "API key is disabled",
            });
        }

        if let Some(expires_at) = api_key.expires_at
            && expires_at < Utc::now()
        {
            return Err(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "KEY_EXPIRED",
                message: "API key has expired",
            });
        }

        let valid = password::verify(token, &api_key.key_hash).map_err(|_| ApiKeyRejection {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: "Failed to verify API key",
        })?;

        if !valid {
            return Err(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "INVALID_TOKEN",
                message: "Invalid API key",
            });
        }

        let _ = store.touch_api_key(api_key.id).await;

        let owner = store
            .find_user_by_id(api_key.user_id)
            .await
            .map_err(|_| ApiKeyRejection {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "INTERNAL_ERROR",
                message: "Failed to look up API key owner",
            })?;
        let owner_is_admin = owner.map(|u| u.is_admin).unwrap_or(false);

        Ok(ApiKeyAuth {
            key_id: api_key.id,
            user_id: api_key.user_id,
            key_name: api_key.name,
            scopes: api_key.scopes,
            owner_is_admin,
        })
    }
}

// ---------------------------------------------------------------------------
// Authenticated (dual: API key OR JWT)
// ---------------------------------------------------------------------------

/// An authenticated caller, either via API key or JWT.
///
/// If the `Authorization: Bearer` token starts with `irfl_`, API key auth is used.
/// Otherwise, JWT auth is attempted (cookie first, then header).
///
/// # Examples
///
/// ```no_run
/// use ironflow_auth::extractor::Authenticated;
///
/// async fn protected(auth: Authenticated) -> String {
///     format!("User {}", auth.user_id)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Authenticated {
    /// The authenticated user's ID.
    pub user_id: Uuid,
    /// The authentication method used.
    pub method: AuthMethod,
}

/// How the caller was authenticated.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Authenticated via JWT (cookie or Bearer header).
    Jwt {
        /// The user's username.
        username: String,
        /// Whether the user is an admin.
        is_admin: bool,
    },
    /// Authenticated via API key.
    ApiKey {
        /// The API key ID.
        key_id: Uuid,
        /// The API key name.
        key_name: String,
        /// Scopes granted to this key.
        scopes: Vec<ApiKeyScope>,
        /// Whether the key owner is an admin (checked at request time).
        owner_is_admin: bool,
    },
}

impl Authenticated {
    /// Whether the authenticated caller has admin privileges.
    ///
    /// For JWT users, checks the `is_admin` claim.
    /// For API key users, checks the owner's current admin status
    /// (fetched at request time, so demotions take effect immediately).
    pub fn is_admin(&self) -> bool {
        match &self.method {
            AuthMethod::Jwt { is_admin, .. } => *is_admin,
            AuthMethod::ApiKey { owner_is_admin, .. } => *owner_is_admin,
        }
    }
}

impl<S> FromRequestParts<S> for Authenticated
where
    S: Send + Sync,
    Arc<JwtConfig>: FromRef<S>,
    Arc<dyn Store>: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let cookie_token = jar.get(AUTH_COOKIE_NAME).map(|c| c.value().to_string());

        let header_token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.to_string());

        // If the Bearer token is an API key, use API key auth
        if let Some(ref token) = header_token
            && token.starts_with(API_KEY_PREFIX)
        {
            let api_key_auth =
                ApiKeyAuth::from_request_parts(parts, state)
                    .await
                    .map_err(|_| AuthRejection {
                        status: StatusCode::UNAUTHORIZED,
                        code: "INVALID_TOKEN",
                        message: "Invalid or expired authentication token",
                    })?;
            return Ok(Authenticated {
                user_id: api_key_auth.user_id,
                method: AuthMethod::ApiKey {
                    key_id: api_key_auth.key_id,
                    key_name: api_key_auth.key_name,
                    scopes: api_key_auth.scopes,
                    owner_is_admin: api_key_auth.owner_is_admin,
                },
            });
        }

        // Otherwise, try JWT (cookie first, then header)
        let token = cookie_token.or(header_token).ok_or(AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "MISSING_TOKEN",
            message: "No authentication token provided",
        })?;

        let jwt_config = Arc::<JwtConfig>::from_ref(state);
        let claims = AccessToken::decode(&token, &jwt_config).map_err(|_| AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "INVALID_TOKEN",
            message: "Invalid or expired authentication token",
        })?;

        Ok(Authenticated {
            user_id: claims.user_id,
            method: AuthMethod::Jwt {
                username: claims.username,
                is_admin: claims.is_admin,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::FromRef;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::{Json, Router};
    use http_body_util::BodyExt;
    use ironflow_store::entities::{ApiKeyScope, NewApiKey};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use ironflow_store::entities::NewUser;
    use serde_json::Value;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::jwt::{AccessToken, JwtConfig};
    use crate::password;

    use super::*;

    #[derive(Clone)]
    struct TestState {
        jwt_config: Arc<JwtConfig>,
        store: Arc<dyn Store>,
    }

    impl FromRef<TestState> for Arc<JwtConfig> {
        fn from_ref(state: &TestState) -> Self {
            state.jwt_config.clone()
        }
    }

    impl FromRef<TestState> for Arc<dyn Store> {
        fn from_ref(state: &TestState) -> Self {
            state.store.clone()
        }
    }

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret-key-for-extractor-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    fn test_state() -> TestState {
        TestState {
            jwt_config: test_jwt_config(),
            store: Arc::new(InMemoryStore::new()),
        }
    }

    async fn response_json(resp: axum::http::Response<Body>) -> Value {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    // ---------------------------------------------------------------
    // AuthenticatedUser (JWT only)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn jwt_extractor_from_bearer_header() {
        let state = test_state();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "alice", false, &state.jwt_config).unwrap();

        let app = Router::new()
            .route(
                "/me",
                get(|user: AuthenticatedUser| async move {
                    Json(json!({
                        "user_id": user.user_id,
                        "username": user.username,
                        "is_admin": user.is_admin
                    }))
                }),
            )
            .with_state(state);

        let req = Request::builder()
            .uri("/me")
            .header("authorization", format!("Bearer {}", token.0))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = response_json(resp).await;
        assert_eq!(json["user_id"], user_id.to_string());
        assert_eq!(json["username"], "alice");
        assert_eq!(json["is_admin"], false);
    }

    #[tokio::test]
    async fn jwt_extractor_from_cookie() {
        let state = test_state();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "bob", true, &state.jwt_config).unwrap();

        let app = Router::new()
            .route(
                "/me",
                get(|user: AuthenticatedUser| async move {
                    Json(json!({ "username": user.username, "is_admin": user.is_admin }))
                }),
            )
            .with_state(state);

        let req = Request::builder()
            .uri("/me")
            .header("cookie", format!("{}={}", AUTH_COOKIE_NAME, token.0))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = response_json(resp).await;
        assert_eq!(json["username"], "bob");
        assert_eq!(json["is_admin"], true);
    }

    #[tokio::test]
    async fn jwt_extractor_rejects_missing_token() {
        let app = Router::new()
            .route(
                "/me",
                get(|_user: AuthenticatedUser| async { "ok" }),
            )
            .with_state(test_state());

        let req = Request::builder()
            .uri("/me")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let json = response_json(resp).await;
        assert_eq!(json["error"]["code"], "MISSING_TOKEN");
    }

    #[tokio::test]
    async fn jwt_extractor_rejects_invalid_token() {
        let app = Router::new()
            .route(
                "/me",
                get(|_user: AuthenticatedUser| async { "ok" }),
            )
            .with_state(test_state());

        let req = Request::builder()
            .uri("/me")
            .header("authorization", "Bearer invalid.token.here")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let json = response_json(resp).await;
        assert_eq!(json["error"]["code"], "INVALID_TOKEN");
    }

    // ---------------------------------------------------------------
    // ApiKeyAuth
    // ---------------------------------------------------------------

    async fn setup_api_key(store: &Arc<dyn Store>) -> (Uuid, String) {
        let user = store
            .create_user(NewUser {
                email: "key-owner@test.com".to_string(),
                username: "keyowner".to_string(),
                password_hash: password::hash("pass123").unwrap(),
                is_admin: Some(true),
            })
            .await
            .unwrap();

        let raw_key = "irfl_abcdef12rest-of-secret-key";
        let key_hash = password::hash(raw_key).unwrap();
        let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];

        store
            .create_api_key(NewApiKey {
                user_id: user.id,
                name: "test-key".to_string(),
                key_hash,
                key_prefix: prefix.to_string(),
                scopes: vec![ApiKeyScope::RunsRead, ApiKeyScope::WorkflowsRead],
                expires_at: None,
            })
            .await
            .unwrap();

        (user.id, raw_key.to_string())
    }

    #[tokio::test]
    async fn api_key_extractor_valid_key() {
        let state = test_state();
        let (user_id, raw_key) = setup_api_key(&state.store).await;

        let app = Router::new()
            .route(
                "/check",
                get(|key: ApiKeyAuth| async move {
                    Json(json!({
                        "user_id": key.user_id,
                        "key_name": key.key_name,
                        "scopes": key.scopes,
                        "owner_is_admin": key.owner_is_admin
                    }))
                }),
            )
            .with_state(state);

        let req = Request::builder()
            .uri("/check")
            .header("authorization", format!("Bearer {raw_key}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = response_json(resp).await;
        assert_eq!(json["user_id"], user_id.to_string());
        assert_eq!(json["key_name"], "test-key");
        assert_eq!(json["owner_is_admin"], true);
    }

    #[tokio::test]
    async fn api_key_extractor_rejects_missing_header() {
        let app = Router::new()
            .route(
                "/check",
                get(|_key: ApiKeyAuth| async { "ok" }),
            )
            .with_state(test_state());

        let req = Request::builder()
            .uri("/check")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let json = response_json(resp).await;
        assert_eq!(json["error"]["code"], "MISSING_TOKEN");
    }

    #[tokio::test]
    async fn api_key_extractor_rejects_non_irfl_token() {
        let app = Router::new()
            .route(
                "/check",
                get(|_key: ApiKeyAuth| async { "ok" }),
            )
            .with_state(test_state());

        let req = Request::builder()
            .uri("/check")
            .header("authorization", "Bearer not-an-api-key")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let json = response_json(resp).await;
        assert_eq!(json["error"]["code"], "INVALID_TOKEN");
    }

    #[tokio::test]
    async fn api_key_extractor_rejects_unknown_key() {
        let app = Router::new()
            .route(
                "/check",
                get(|_key: ApiKeyAuth| async { "ok" }),
            )
            .with_state(test_state());

        let req = Request::builder()
            .uri("/check")
            .header("authorization", "Bearer irfl_unknown1rest-of-key")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let json = response_json(resp).await;
        assert_eq!(json["error"]["code"], "INVALID_TOKEN");
    }

    // ---------------------------------------------------------------
    // ApiKeyAuth::has_scope()
    // ---------------------------------------------------------------

    #[test]
    fn has_scope_returns_true_for_granted_scope() {
        let auth = ApiKeyAuth {
            key_id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            key_name: "k".to_string(),
            scopes: vec![ApiKeyScope::RunsRead, ApiKeyScope::WorkflowsRead],
            owner_is_admin: false,
        };
        assert!(auth.has_scope(&ApiKeyScope::RunsRead));
        assert!(auth.has_scope(&ApiKeyScope::WorkflowsRead));
    }

    #[test]
    fn has_scope_returns_false_for_missing_scope() {
        let auth = ApiKeyAuth {
            key_id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            key_name: "k".to_string(),
            scopes: vec![ApiKeyScope::RunsRead],
            owner_is_admin: false,
        };
        assert!(!auth.has_scope(&ApiKeyScope::RunsWrite));
        assert!(!auth.has_scope(&ApiKeyScope::Admin));
    }

    #[test]
    fn has_scope_admin_grants_everything() {
        let auth = ApiKeyAuth {
            key_id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            key_name: "k".to_string(),
            scopes: vec![ApiKeyScope::Admin],
            owner_is_admin: true,
        };
        assert!(auth.has_scope(&ApiKeyScope::RunsRead));
        assert!(auth.has_scope(&ApiKeyScope::RunsWrite));
        assert!(auth.has_scope(&ApiKeyScope::StatsRead));
    }

    // ---------------------------------------------------------------
    // Authenticated (dual auth)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn authenticated_via_jwt() {
        let state = test_state();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "alice", true, &state.jwt_config).unwrap();

        let app = Router::new()
            .route(
                "/auth",
                get(|auth: Authenticated| async move {
                    Json(json!({
                        "user_id": auth.user_id,
                        "is_admin": auth.is_admin(),
                        "method": match auth.method {
                            AuthMethod::Jwt { .. } => "jwt",
                            AuthMethod::ApiKey { .. } => "api_key",
                        }
                    }))
                }),
            )
            .with_state(state);

        let req = Request::builder()
            .uri("/auth")
            .header("authorization", format!("Bearer {}", token.0))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = response_json(resp).await;
        assert_eq!(json["user_id"], user_id.to_string());
        assert_eq!(json["is_admin"], true);
        assert_eq!(json["method"], "jwt");
    }

    #[tokio::test]
    async fn authenticated_via_api_key() {
        let state = test_state();
        let (user_id, raw_key) = setup_api_key(&state.store).await;

        let app = Router::new()
            .route(
                "/auth",
                get(|auth: Authenticated| async move {
                    Json(json!({
                        "user_id": auth.user_id,
                        "is_admin": auth.is_admin(),
                        "method": match auth.method {
                            AuthMethod::Jwt { .. } => "jwt",
                            AuthMethod::ApiKey { .. } => "api_key",
                        }
                    }))
                }),
            )
            .with_state(state);

        let req = Request::builder()
            .uri("/auth")
            .header("authorization", format!("Bearer {raw_key}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = response_json(resp).await;
        assert_eq!(json["user_id"], user_id.to_string());
        assert_eq!(json["is_admin"], true);
        assert_eq!(json["method"], "api_key");
    }

    #[tokio::test]
    async fn authenticated_rejects_missing_token() {
        let app = Router::new()
            .route(
                "/auth",
                get(|_auth: Authenticated| async { "ok" }),
            )
            .with_state(test_state());

        let req = Request::builder()
            .uri("/auth")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let json = response_json(resp).await;
        assert_eq!(json["error"]["code"], "MISSING_TOKEN");
    }

    // ---------------------------------------------------------------
    // Authenticated::is_admin()
    // ---------------------------------------------------------------

    #[test]
    fn is_admin_jwt_true() {
        let auth = Authenticated {
            user_id: Uuid::now_v7(),
            method: AuthMethod::Jwt {
                username: "admin".to_string(),
                is_admin: true,
            },
        };
        assert!(auth.is_admin());
    }

    #[test]
    fn is_admin_jwt_false() {
        let auth = Authenticated {
            user_id: Uuid::now_v7(),
            method: AuthMethod::Jwt {
                username: "user".to_string(),
                is_admin: false,
            },
        };
        assert!(!auth.is_admin());
    }

    #[test]
    fn is_admin_api_key_true() {
        let auth = Authenticated {
            user_id: Uuid::now_v7(),
            method: AuthMethod::ApiKey {
                key_id: Uuid::now_v7(),
                key_name: "k".to_string(),
                scopes: vec![],
                owner_is_admin: true,
            },
        };
        assert!(auth.is_admin());
    }

    #[test]
    fn is_admin_api_key_false() {
        let auth = Authenticated {
            user_id: Uuid::now_v7(),
            method: AuthMethod::ApiKey {
                key_id: Uuid::now_v7(),
                key_name: "k".to_string(),
                scopes: vec![],
                owner_is_admin: false,
            },
        };
        assert!(!auth.is_admin());
    }

    // ---------------------------------------------------------------
    // AuthRejection / ApiKeyRejection IntoResponse
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn auth_rejection_into_response() {
        let rejection = AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "TEST_CODE",
            message: "test message",
        };
        let resp = rejection.into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "TEST_CODE");
        assert_eq!(json["error"]["message"], "test message");
    }

    #[tokio::test]
    async fn api_key_rejection_into_response() {
        let rejection = ApiKeyRejection {
            status: StatusCode::FORBIDDEN,
            code: "KEY_DISABLED",
            message: "API key is disabled",
        };
        let resp = rejection.into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "KEY_DISABLED");
        assert_eq!(json["error"]["message"], "API key is disabled");
    }
}
