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
use ironflow_auth::extractor::API_KEY_PREFIX;

/// Request body for creating an API key.
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

    let raw_key = generate_api_key();
    let key_prefix = raw_key[..API_KEY_PREFIX.len() + 8].to_string();
    let key_hash =
        password::hash(&raw_key).map_err(|e| ApiError::Internal(format!("hashing: {e}")))?;

    let api_key = state
        .api_key_store
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
