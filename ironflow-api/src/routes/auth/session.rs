//! Session issuance shared by sign-in, sign-up, refresh and password change.

use axum::http::{HeaderMap, HeaderValue};
use chrono::{Duration, Utc};

use ironflow_auth::cookies::{build_auth_cookie, build_refresh_cookie};
use ironflow_auth::jwt::{AccessToken, RefreshToken, token_hash};
use ironflow_store::entities::{NewRefreshToken, User};

use crate::error::ApiError;
use crate::state::AppState;

/// Mint an access/refresh pair for `user` and record the refresh token.
///
/// Both tokens carry the user's current role and `token_version`, read from
/// `user`. Only the SHA-256 hash of the refresh token is stored, so it can be
/// consumed once by the refresh route. Returns the two `Set-Cookie` headers.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if a token cannot be signed, and
/// [`ApiError::Store`] if the refresh token cannot be recorded.
pub(crate) async fn issue_session(state: &AppState, user: &User) -> Result<HeaderMap, ApiError> {
    let access = AccessToken::for_user_with_version(
        user.id,
        &user.username,
        user.is_admin,
        user.token_version,
        &state.jwt_config,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let refresh = RefreshToken::for_user_with_version(
        user.id,
        &user.username,
        user.is_admin,
        user.token_version,
        &state.jwt_config,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    state
        .store
        .store_refresh_token(NewRefreshToken {
            token_hash: token_hash(&refresh.0),
            user_id: user.id,
            expires_at: Utc::now() + Duration::seconds(state.jwt_config.refresh_token_ttl_secs),
        })
        .await?;

    let mut headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&build_auth_cookie(&access.0, &state.jwt_config)) {
        headers.append("Set-Cookie", val);
    }
    if let Ok(val) = HeaderValue::from_str(&build_refresh_cookie(&refresh.0, &state.jwt_config)) {
        headers.append("Set-Cookie", val);
    }

    Ok(headers)
}
