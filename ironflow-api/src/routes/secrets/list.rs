//! `GET /api/v1/secrets` -- List secrets (admin only, never exposes values).

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;

use ironflow_auth::extractor::Authenticated;

use crate::entities::SecretResponse;
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// Query parameters for listing secrets.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[derive(Debug, Deserialize)]
pub struct ListSecretsQuery {
    /// Filter by key prefix (e.g. `workflows/inbox/`).
    pub prefix: Option<String>,
    /// Page number (1-based, default 1).
    pub page: Option<u32>,
    /// Items per page (default 50, max 100).
    pub per_page: Option<u32>,
}

/// List secret metadata. Admin only.
///
/// Returns key, id, and timestamps. Never returns encrypted or decrypted values.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/secrets",
        tags = ["secrets"],
        params(ListSecretsQuery),
        responses(
            (status = 200, description = "Secrets listed", body = Vec<SecretResponse>),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_secrets(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<ListSecretsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let prefix = query.prefix.as_deref().unwrap_or("");
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 100);

    let result = state.store.list_secrets(prefix, page, per_page).await?;

    let data: Vec<SecretResponse> = result.items.into_iter().map(SecretResponse::from).collect();

    Ok(ok_paged(data, result.page, result.per_page, result.total))
}
