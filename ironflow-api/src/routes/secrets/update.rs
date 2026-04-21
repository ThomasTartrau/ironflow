//! `PUT /api/v1/secrets/:key` -- Update a secret value (admin only).

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;

use crate::entities::SecretResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Request body for updating a secret value.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateSecretRequest {
    /// New secret value (plaintext, will be encrypted at rest).
    #[validate(length(min = 1, max = 65536))]
    pub value: String,
}

/// Update a secret's value. Admin only.
///
/// The key is taken from the URL path. The response never includes
/// the secret value.
///
/// # Errors
///
/// - 403 if the caller is not an admin
/// - 404 if the secret key does not exist
/// - 400 if the value is invalid
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        put,
        path = "/api/v1/secrets/{key}",
        tags = ["secrets"],
        params(("key" = String, Path, description = "Secret key")),
        request_body(content = UpdateSecretRequest, description = "New secret value"),
        responses(
            (status = 200, description = "Secret updated", body = SecretResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Secret not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn update_secret(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(req): Json<UpdateSecretRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let existing = state.store.get_secret(&key).await?;
    if existing.is_none() {
        return Err(ApiError::SecretNotFound(key));
    }

    let secret = state.store.set_secret(&key, &req.value).await?;

    Ok(ok(SecretResponse::from(secret)))
}
