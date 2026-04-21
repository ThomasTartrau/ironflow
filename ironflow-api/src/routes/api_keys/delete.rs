//! `DELETE /api/v1/api-keys/:id` -- Delete an API key.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use ironflow_auth::extractor::AuthenticatedUser;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

/// Delete an API key owned by the authenticated user.
///
/// # Errors
///
/// - 404 if the key does not exist or belongs to another user
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/api-keys/{id}",
        tags = ["api-keys"],
        params(
            ("id" = Uuid, Path, description = "API key ID")
        ),
        responses(
            (status = 204, description = "API key deleted successfully"),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "API key not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn delete_api_key(
    user: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let key = state
        .store
        .find_api_key_by_id(id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::ApiKeyNotFound(id))?;

    if key.user_id != user.user_id {
        return Err(ApiError::ApiKeyNotFound(id));
    }

    state
        .store
        .delete_api_key(id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}
