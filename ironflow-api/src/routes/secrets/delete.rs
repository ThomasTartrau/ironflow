//! `DELETE /api/v1/secrets/:key` -- Delete a secret (admin only).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use ironflow_auth::extractor::Authenticated;

use crate::error::ApiError;
use crate::state::AppState;

/// Delete a secret by key. Admin only.
///
/// # Errors
///
/// - 403 if the caller is not an admin
/// - 404 if the secret key does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/secrets/{key}",
        tags = ["secrets"],
        params(("key" = String, Path, description = "Secret key")),
        responses(
            (status = 204, description = "Secret deleted"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Secret not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn delete_secret(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let deleted = state.store.delete_secret(&key).await?;
    if !deleted {
        return Err(ApiError::SecretNotFound(key));
    }

    Ok(StatusCode::NO_CONTENT)
}
