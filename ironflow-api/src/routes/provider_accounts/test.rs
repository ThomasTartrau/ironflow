//! `POST /api/v1/provider-accounts/{id}/test` -- Check a stored credential (admin only).

use axum::extract::{Path, State};
use axum::response::IntoResponse;

use ironflow_auth::extractor::Authenticated;

#[cfg(feature = "openapi")]
use crate::entities::ProviderAccountTestResponse;
use crate::error::ApiError;
use crate::provider_accounts::{self, AccountScope};
use crate::response::ok;
use crate::state::AppState;

/// Check the stored credential against the provider and record the windows
/// it reports. Admin only.
///
/// `result` is `valid`, `limited` or `unauthorized`; an `unauthorized`
/// account is marked as having an invalid token until it is replaced.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/provider-accounts/{id}/test",
        tags = ["provider-accounts"],
        params(("id" = String, Path, description = "Account UUID or name")),
        responses(
            (status = 200, description = "Credential checked", body = ProviderAccountTestResponse),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Provider account not found"),
            (status = 422, description = "No credential stored"),
            (status = 502, description = "Provider unreachable")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn test_provider_account(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    provider_accounts::authorize(&auth, AccountScope::Manage)?;
    let account = provider_accounts::resolve(state.store.as_ref(), &id).await?;
    let result = provider_accounts::test(&state, &account).await?;
    Ok(ok(result))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use ironflow_core::account::ClaudeSubscriptionKind;

    use crate::routes::provider_accounts::test_support::{
        Stub, bearer, call, create_account, state_with_stub, stub_anthropic,
    };

    #[tokio::test]
    async fn provider_accounts_test_route_results() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        let auth = bearer(&state, true);
        let uri = "/api/v1/provider-accounts/perso-max/test";

        let (status, resp, _) = call(&state, "POST", uri, &auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"]["result"], "valid");
        assert_eq!(resp["data"]["windows"].as_array().unwrap().len(), 2);

        // Same store, provider now answering 429.
        let limited =
            state
                .clone()
                .with_account_kind(Arc::new(ClaudeSubscriptionKind::with_api_base(
                    &stub_anthropic(Stub::Limited).await,
                )));
        let (status, resp, _) = call(&limited, "POST", uri, &auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"]["result"], "limited");

        // Provider now rejecting the token: the account is flagged.
        let rejected =
            state
                .clone()
                .with_account_kind(Arc::new(ClaudeSubscriptionKind::with_api_base(
                    &stub_anthropic(Stub::Unauthorized).await,
                )));
        let (status, resp, _) = call(&rejected, "POST", uri, &auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"]["result"], "unauthorized");
        let (_, account, _) = call(
            &state,
            "GET",
            "/api/v1/provider-accounts/perso-max",
            &auth,
            None,
        )
        .await;
        assert_eq!(account["data"]["state"], "token_invalid");

        // Unreachable provider.
        let down =
            state
                .clone()
                .with_account_kind(Arc::new(ClaudeSubscriptionKind::with_api_base(
                    "http://127.0.0.1:1",
                )));
        let (status, _, _) = call(&down, "POST", uri, &auth, None).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }
}
