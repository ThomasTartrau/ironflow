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
    /// Creation date.
    pub created_at: DateTime<Utc>,
}

/// Create a new API key for the authenticated user.
///
/// # Errors
///
/// - 400 if the name is empty or scopes are invalid
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
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".to_string()));
    }

    if req.scopes.is_empty() {
        return Err(ApiError::BadRequest(
            "at least one scope is required".to_string(),
        ));
    }

    if !user.is_admin && !ApiKeyScope::all_allowed_for_member(&req.scopes) {
        return Err(ApiError::Forbidden);
    }

    let raw_key = generate_api_key();
    let key_prefix = raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN].to_string();
    let key_hash =
        password::hash(&raw_key).map_err(|e| ApiError::Internal(format!("hashing: {e}")))?;

    let api_key = state
        .store
        .create_api_key(NewApiKey {
            user_id: user.user_id,
            name: req.name,
            key_hash,
            key_prefix: key_prefix.clone(),
            scopes: req.scopes,
            expires_at: req.expires_at,
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
        created_at: api_key.created_at,
    };

    Ok((StatusCode::CREATED, ok(response)))
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
