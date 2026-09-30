//! `GET /api/v1/provider-accounts/{id}` -- Get one Provider Account (admin only).

use axum::extract::{Path, State};
use axum::response::IntoResponse;

use ironflow_auth::extractor::Authenticated;

#[cfg(feature = "openapi")]
use crate::entities::ProviderAccountResponse;
use crate::error::ApiError;
use crate::provider_accounts::{self, AccountScope};
use crate::response::ok;
use crate::state::AppState;

/// Get a Provider Account by UUID or name. Admin only.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/provider-accounts/{id}",
        tags = ["provider-accounts"],
        params(("id" = String, Path, description = "Account UUID or name")),
        responses(
            (status = 200, description = "Provider account", body = ProviderAccountResponse),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Provider account not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_provider_account(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    provider_accounts::authorize(&auth, AccountScope::Read)?;
    let account = provider_accounts::resolve(state.store.as_ref(), &id).await?;
    let response = provider_accounts::to_response(state.store.as_ref(), account).await?;
    Ok(ok(response))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::routes::provider_accounts::test_support::{
        Stub, bearer, call, create_account, state_with_stub,
    };

    #[tokio::test]
    async fn provider_accounts_get_by_name_and_id() {
        let state = state_with_stub(Stub::Valid).await;
        let created = create_account(&state, "perso-max").await;
        let id = created["data"]["id"].as_str().unwrap().to_string();
        let auth = bearer(&state, true);

        let (status, by_name, _) = call(
            &state,
            "GET",
            "/api/v1/provider-accounts/perso-max",
            &auth,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(by_name["data"]["id"], id.as_str());

        let (status, by_id, _) = call(
            &state,
            "GET",
            &format!("/api/v1/provider-accounts/{id}"),
            &auth,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(by_id["data"]["name"], "perso-max");

        let (status, missing, _) =
            call(&state, "GET", "/api/v1/provider-accounts/nope", &auth, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(missing["error"]["code"], "PROVIDER_ACCOUNT_NOT_FOUND");
    }
}
