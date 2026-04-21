//! `GET /api/v1/internal/secrets/:key` -- Get a secret by key (worker only).

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use ironflow_store::entities::Secret;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Internal secret response that INCLUDES the decrypted value.
///
/// Unlike the public `SecretResponse`, this is only sent to the worker
/// over the internal API (protected by WORKER_TOKEN).
#[derive(Serialize)]
struct InternalSecretResponse {
    id: Uuid,
    key: String,
    value: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<Secret> for InternalSecretResponse {
    fn from(s: Secret) -> Self {
        Self {
            id: s.id,
            key: s.key,
            value: s.value,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// Get a decrypted secret by key. Worker-only (WORKER_TOKEN auth).
pub async fn get_secret(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let secret = state
        .store
        .get_secret(&key)
        .await?
        .ok_or(ApiError::SecretNotFound(key))?;

    Ok(ok(InternalSecretResponse::from(secret)))
}
