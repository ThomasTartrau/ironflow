//! `DELETE /api/v1/provider-accounts/{id}` -- Delete a Provider Account (admin only).

use axum::extract::{Path, State};
use axum::http::StatusCode;

use ironflow_auth::extractor::Authenticated;

use crate::error::ApiError;
use crate::provider_accounts::{self, AccountScope};
use crate::state::AppState;

/// Delete a Provider Account and its credential. Admin only.
///
/// Its windows and usage history go with it; the steps that ran under it
/// keep their history but lose the link to the account.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/provider-accounts/{id}",
        tags = ["provider-accounts"],
        params(("id" = String, Path, description = "Account UUID or name")),
        responses(
            (status = 204, description = "Provider account deleted"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Provider account not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn delete_provider_account(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    provider_accounts::authorize(&auth, AccountScope::Manage)?;
    let account = provider_accounts::resolve(state.store.as_ref(), &id).await?;
    provider_accounts::delete(&state, auth.user_id, account).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::routes::provider_accounts::test_support::{
        Stub, bearer, call, create_account, state_with_stub,
    };

    #[tokio::test]
    async fn provider_accounts_delete_removes_secret() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        let account = state
            .store
            .find_provider_account_by_name("perso-max")
            .await
            .unwrap()
            .unwrap();
        let auth = bearer(&state, true);

        let (status, _, _) = call(
            &state,
            "DELETE",
            "/api/v1/provider-accounts/perso-max",
            &auth,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(
            state
                .store
                .get_secret(&account.secret_key)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .store
                .get_provider_account(account.id)
                .await
                .unwrap()
                .is_none()
        );

        let (status, _, _) = call(
            &state,
            "DELETE",
            "/api/v1/provider-accounts/perso-max",
            &auth,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
