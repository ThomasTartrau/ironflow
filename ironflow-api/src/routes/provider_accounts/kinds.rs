//! `GET /api/v1/provider-accounts/kinds` -- Supported account kinds (admin only).

use axum::extract::State;
use axum::response::IntoResponse;

use ironflow_auth::extractor::Authenticated;

use crate::entities::{AccountFormFieldResponse, AccountKindResponse};
use crate::error::ApiError;
use crate::provider_accounts::{self, AccountScope};
use crate::response::ok;
use crate::state::AppState;

/// List the account kinds this server supports, with their form fields. Admin only.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/provider-accounts/kinds",
        tags = ["provider-accounts"],
        responses(
            (status = 200, description = "Supported kinds", body = Vec<AccountKindResponse>),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_account_kinds(
    auth: Authenticated,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    provider_accounts::authorize(&auth, AccountScope::Read)?;
    let mut kinds: Vec<AccountKindResponse> = state
        .account_kinds
        .values()
        .map(|kind| AccountKindResponse {
            id: kind.id().to_string(),
            display_name: kind.display_name().to_string(),
            fields: kind
                .form_fields()
                .into_iter()
                .map(AccountFormFieldResponse::from)
                .collect(),
        })
        .collect();
    kinds.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(ok(kinds))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use ironflow_store::entities::ApiKeyScope;

    use crate::routes::provider_accounts::test_support::{
        Stub, api_key_bearer, bearer, call, state_with_stub,
    };

    #[tokio::test]
    async fn provider_accounts_kinds_lists_claude_subscription() {
        let state = state_with_stub(Stub::Valid).await;
        let (status, resp, _) = call(
            &state,
            "GET",
            "/api/v1/provider-accounts/kinds",
            &bearer(&state, true),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"][0]["id"], "claude_subscription");
        assert_eq!(resp["data"][0]["fields"][0]["name"], "token");
        assert_eq!(resp["data"][0]["fields"][0]["secret"], true);
    }

    #[tokio::test]
    async fn provider_accounts_api_key_without_scope_is_rejected() {
        let state = state_with_stub(Stub::Valid).await;
        let uri = "/api/v1/provider-accounts";

        let runs_only = api_key_bearer(&state, vec![ApiKeyScope::RunsRead]).await;
        let (status, resp, _) = call(&state, "GET", uri, &runs_only, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(resp["error"]["code"], "INSUFFICIENT_SCOPE");

        let reader = api_key_bearer(&state, vec![ApiKeyScope::AccountsRead]).await;
        let (status, _, _) = call(&state, "GET", uri, &reader, None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, resp, _) =
            call(&state, "DELETE", &format!("{uri}/perso"), &reader, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(resp["error"]["code"], "INSUFFICIENT_SCOPE");

        let admin_key = api_key_bearer(&state, vec![ApiKeyScope::Admin]).await;
        let (status, _, _) = call(
            &state,
            "GET",
            "/api/v1/provider-accounts/kinds",
            &admin_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}
