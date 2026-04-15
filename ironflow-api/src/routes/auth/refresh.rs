//! `POST /api/v1/auth/refresh` — Refresh access token using a refresh token.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;

use ironflow_auth::cookies::{build_auth_cookie, build_refresh_cookie, extract_refresh_token};
use ironflow_auth::jwt::{AccessToken, RefreshToken};

use crate::error::ApiError;
use crate::state::AppState;

/// Refresh the access token using a valid refresh token from cookies.
///
/// # Errors
///
/// - 401 if the refresh token is missing, invalid, or expired
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/auth/refresh",
        tags = ["auth"],
        responses(
            (status = 204, description = "Access token refreshed successfully, new cookies set"),
            (status = 401, description = "Invalid or expired refresh token")
        )
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let raw_refresh = extract_refresh_token(&headers).ok_or(ApiError::Unauthorized)?;

    let claims = RefreshToken::decode(&raw_refresh, &state.jwt_config)
        .map_err(|_| ApiError::Unauthorized)?;

    let new_access = AccessToken::for_user(
        claims.user_id,
        &claims.username,
        claims.is_admin,
        &state.jwt_config,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let new_refresh = RefreshToken::for_user(
        claims.user_id,
        &claims.username,
        claims.is_admin,
        &state.jwt_config,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&build_auth_cookie(&new_access.0, &state.jwt_config)) {
        response_headers.append("Set-Cookie", val);
    }
    if let Ok(val) = HeaderValue::from_str(&build_refresh_cookie(&new_refresh.0, &state.jwt_config))
    {
        response_headers.append("Set-Cookie", val);
    }

    Ok((StatusCode::NO_CONTENT, response_headers))
}
