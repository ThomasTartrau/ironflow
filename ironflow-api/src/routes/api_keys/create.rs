//! `POST /api/v1/api-keys` -- Create a new API key.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use ironflow_auth::extractor::AuthenticatedUser;
use ironflow_auth::password;
use ironflow_store::entities::{ApiKeyScope, NewApiKey};
use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;
use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};

/// Maximum length of an API key name, in characters (matches the
/// `iam.api_keys.name` VARCHAR(255) column).
const MAX_API_KEY_NAME_LEN: usize = 255;

/// Request body for creating an API key.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    /// Human-readable name for this key.
    pub name: String,
    /// Scopes to grant.
    pub scopes: Vec<ApiKeyScope>,
    /// Optional expiration date (ISO 8601).
    pub expires_at: Option<DateTime<Utc>>,
    /// Optional per-key rate limit override (requests per minute).
    /// When set, the server uses this value instead of the global rate
    /// limit for requests authenticated with this key.
    pub rate_limit_override: Option<u32>,
}

/// Response returned when creating an API key.
/// The raw key is only shown once.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    /// API key ID.
    pub id: Uuid,
    /// The full raw API key (only returned at creation time).
    pub key: String,
    /// First characters for identification.
    pub key_prefix: String,
    /// Key name.
    pub name: String,
    /// Granted scopes.
    pub scopes: Vec<ApiKeyScope>,
    /// Expiration date.
    pub expires_at: Option<DateTime<Utc>>,
    /// Per-key rate limit override (requests per minute), if set.
    pub rate_limit_override: Option<u32>,
    /// Creation date.
    pub created_at: DateTime<Utc>,
}

/// Create a new API key for the authenticated user.
///
/// # Errors
///
/// - 400 if the name is empty or longer than 255 characters (after trimming), or scopes are invalid
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/api-keys",
        tags = ["api-keys"],
        request_body(content = CreateApiKeyRequest, description = "API key configuration"),
        responses(
            (status = 201, description = "API key created successfully", body = CreateApiKeyResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden (member trying to assign forbidden scopes)")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn create_api_key(
    user: AuthenticatedUser,
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let name = validate_name(&req.name)?.to_string();

    if req.scopes.is_empty() {
        return Err(ApiError::BadRequest(
            "at least one scope is required".to_string(),
        ));
    }

    if !user.is_admin && !ApiKeyScope::all_allowed_for_member(&req.scopes) {
        return Err(ApiError::Forbidden);
    }

    if let Some(override_val) = req.rate_limit_override
        && override_val > 10_000
    {
        return Err(ApiError::BadRequest(
            "rate_limit_override must be between 0 and 10000".to_string(),
        ));
    }

    let raw_key = generate_api_key();
    let key_prefix = raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN].to_string();
    let key_hash =
        password::hash(&raw_key).map_err(|e| ApiError::Internal(format!("hashing: {e}")))?;

    let api_key = state
        .store
        .create_api_key(NewApiKey {
            user_id: user.user_id,
            name,
            key_hash,
            key_prefix: key_prefix.clone(),
            scopes: req.scopes,
            expires_at: req.expires_at,
            rate_limit_override: req.rate_limit_override,
        })
        .await
        .map_err(ApiError::from)?;

    let response = CreateApiKeyResponse {
        id: api_key.id,
        key: raw_key,
        key_prefix,
        name: api_key.name,
        scopes: api_key.scopes,
        expires_at: api_key.expires_at,
        rate_limit_override: api_key.rate_limit_override,
        created_at: api_key.created_at,
    };

    Ok((StatusCode::CREATED, ok(response)))
}

/// Trim `name` and check it is non-empty and within the column bound.
///
/// The trimmed value is the one to store, so the stored name always fits.
/// Characters are counted, not bytes, as Postgres does for VARCHAR(n).
fn validate_name(name: &str) -> Result<&str, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".to_string()));
    }
    if name.chars().count() > MAX_API_KEY_NAME_LEN {
        return Err(ApiError::BadRequest(format!(
            "name must be at most {MAX_API_KEY_NAME_LEN} characters"
        )));
    }
    Ok(name)
}

/// Generate a random API key with the `irfl_` prefix.
fn generate_api_key() -> String {
    let mut rng = rand::rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    let encoded = hex::encode(&random_bytes);
    format!("{API_KEY_PREFIX}{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use ironflow_auth::jwt::JwtConfig;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::NewUser;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use tokio::sync::broadcast;

    async fn test_state() -> (AppState, AuthenticatedUser) {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-api-keys".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store.clone(),
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        );

        let user = store
            .create_user(NewUser {
                email: "admin@example.com".to_string(),
                username: "admin".to_string(),
                password_hash: password::hash("password123").expect("hash"),
                is_admin: Some(true),
            })
            .await
            .expect("create user");
        let auth = AuthenticatedUser {
            user_id: user.id,
            username: user.username,
            is_admin: true,
        };
        (state, auth)
    }

    fn request(name: String) -> CreateApiKeyRequest {
        CreateApiKeyRequest {
            name,
            scopes: vec![ApiKeyScope::RunsRead],
            expires_at: None,
            rate_limit_override: None,
        }
    }

    #[test]
    fn validate_name_rejects_empty_and_whitespace() {
        assert!(matches!(validate_name(""), Err(ApiError::BadRequest(_))));
        assert!(matches!(validate_name("   "), Err(ApiError::BadRequest(_))));
    }

    #[test]
    fn validate_name_accepts_exactly_255_chars() {
        let name = "a".repeat(255);
        assert_eq!(validate_name(&name).expect("valid"), name);
    }

    #[test]
    fn validate_name_rejects_256_chars() {
        let name = "a".repeat(256);
        match validate_name(&name) {
            Err(ApiError::BadRequest(msg)) => assert!(msg.contains("255")),
            _ => panic!("expected BadRequest"),
        }
    }

    #[test]
    fn validate_name_trims_before_counting() {
        let padded = format!("  {}  ", "a".repeat(255));
        let name = validate_name(&padded).expect("valid");
        assert_eq!(name.len(), 255);
    }

    #[test]
    fn validate_name_counts_chars_not_bytes() {
        let ok = "é".repeat(255);
        assert!(validate_name(&ok).is_ok());
        let too_long = "é".repeat(256);
        assert!(matches!(
            validate_name(&too_long),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn create_api_key_name_longer_than_255_returns_400() {
        let (state, auth) = test_state().await;
        let user_id = auth.user_id;
        let store = state.store.clone();

        let result = create_api_key(auth, State(state), Json(request("a".repeat(300)))).await;
        assert!(matches!(result, Err(ApiError::BadRequest(_))));

        let keys = store
            .list_api_keys_by_user(user_id)
            .await
            .expect("list keys");
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn create_api_key_name_of_255_chars_returns_201() {
        let (state, auth) = test_state().await;

        let response = create_api_key(auth, State(state), Json(request("a".repeat(255))))
            .await
            .expect("created")
            .into_response();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_api_key_stores_trimmed_name() {
        let (state, auth) = test_state().await;
        let user_id = auth.user_id;
        let store = state.store.clone();

        let response = create_api_key(auth, State(state), Json(request("  key  ".to_string())))
            .await
            .expect("created")
            .into_response();
        assert_eq!(response.status(), StatusCode::CREATED);

        let keys = store
            .list_api_keys_by_user(user_id)
            .await
            .expect("list keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].name, "key");
    }

    #[test]
    fn generate_api_key_has_correct_prefix() {
        let key = generate_api_key();
        assert!(key.starts_with(API_KEY_PREFIX));
    }

    #[test]
    fn generate_api_key_is_long_enough_for_prefix_extraction() {
        let key = generate_api_key();
        let expected_prefix_len = API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN;
        assert!(
            key.len() >= expected_prefix_len,
            "key length {} is shorter than expected prefix length {}",
            key.len(),
            expected_prefix_len
        );

        let prefix = &key[..expected_prefix_len];
        assert_eq!(prefix.len(), 13);
        assert!(prefix.starts_with(API_KEY_PREFIX));
    }

    #[test]
    fn generate_api_key_prefix_fits_varchar_16() {
        let key = generate_api_key();
        let prefix = &key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];
        assert!(
            prefix.len() <= 16,
            "prefix length {} exceeds VARCHAR(16)",
            prefix.len()
        );
    }

    #[test]
    fn generate_api_key_produces_unique_keys() {
        let key1 = generate_api_key();
        let key2 = generate_api_key();
        assert_ne!(key1, key2);
    }
}
