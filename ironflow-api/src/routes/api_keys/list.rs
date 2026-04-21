//! `GET /api/v1/api-keys` -- List API keys for the authenticated user.

use axum::extract::State;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use ironflow_auth::extractor::AuthenticatedUser;
use ironflow_store::entities::ApiKeyScope;
use serde::Serialize;
use uuid::Uuid;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// API key summary (never includes the hash or raw key).
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    /// API key ID.
    pub id: Uuid,
    /// Key name.
    pub name: String,
    /// First characters for identification.
    pub key_prefix: String,
    /// Granted scopes.
    pub scopes: Vec<ApiKeyScope>,
    /// Whether the key is active.
    pub is_active: bool,
    /// Expiration date.
    pub expires_at: Option<DateTime<Utc>>,
    /// Last used date.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Creation date.
    pub created_at: DateTime<Utc>,
}

/// List all API keys for the authenticated user.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/api-keys",
        tags = ["api-keys"],
        responses(
            (status = 200, description = "List of API keys", body = Vec<ApiKeyResponse>),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_api_keys(
    user: AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let keys = state
        .store
        .list_api_keys_by_user(user.user_id)
        .await
        .map_err(ApiError::from)?;

    let responses: Vec<ApiKeyResponse> = keys
        .into_iter()
        .map(|k| ApiKeyResponse {
            id: k.id,
            name: k.name,
            key_prefix: k.key_prefix,
            scopes: k.scopes,
            is_active: k.is_active,
            expires_at: k.expires_at,
            last_used_at: k.last_used_at,
            created_at: k.created_at,
        })
        .collect();

    Ok(ok(responses))
}
