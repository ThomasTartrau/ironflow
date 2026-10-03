//! `GET /api/v1/provider-accounts` -- List Provider Accounts (admin only).

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use chrono::Utc;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::ProviderAccountWindow;

use crate::entities::{ListProviderAccountsQuery, ProviderAccountResponse};
use crate::error::ApiError;
use crate::provider_accounts::{self, AccountScope};
use crate::response::ok_paged;
use crate::state::AppState;

/// List Provider Accounts with their latest windows. Admin only.
///
/// The response never includes the credential.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/provider-accounts",
        tags = ["provider-accounts"],
        params(ListProviderAccountsQuery),
        responses(
            (status = 200, description = "Provider accounts listed", body = Vec<ProviderAccountResponse>),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_provider_accounts(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<ListProviderAccountsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    provider_accounts::authorize(&auth, AccountScope::Read)?;

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 100);
    let result = state
        .store
        .list_provider_accounts(query.kind, page, per_page)
        .await?;

    // One batched query for every account's windows.
    let ids: Vec<Uuid> = result.items.iter().map(|a| a.id).collect();
    let mut windows_by_account: HashMap<Uuid, Vec<ProviderAccountWindow>> = HashMap::new();
    for window in state.store.list_provider_account_windows(ids).await? {
        windows_by_account
            .entry(window.account_id)
            .or_default()
            .push(window);
    }

    let now = Utc::now();
    let data: Vec<ProviderAccountResponse> = result
        .items
        .into_iter()
        .map(|account| {
            let windows = windows_by_account.remove(&account.id).unwrap_or_default();
            let account_state = provider_accounts::account_state(&account, &windows, now);
            ProviderAccountResponse::new(account, windows, account_state)
        })
        .collect();

    Ok(ok_paged(data, result.page, result.per_page, result.total))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::routes::provider_accounts::test_support::{
        Stub, TOKEN, bearer, call, create_account, state_with_stub,
    };

    #[tokio::test]
    async fn provider_accounts_list_never_contains_token() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        create_account(&state, "team-pro").await;

        let (status, body, text) = call(
            &state,
            "GET",
            "/api/v1/provider-accounts",
            &bearer(&state, true).await,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["meta"]["total"], 2);
        assert_eq!(body["data"][0]["state"], "ok");
        assert_eq!(body["data"][0]["windows"].as_array().unwrap().len(), 2);
        assert!(!text.contains(TOKEN));
        assert!(!text.contains("sk-ant-"));
        assert!(!text.contains("accounts/"));
    }

    #[tokio::test]
    async fn provider_accounts_list_filters_by_kind() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        let (status, body, _) = call(
            &state,
            "GET",
            "/api/v1/provider-accounts?kind=other",
            &bearer(&state, true).await,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["meta"]["total"], 0);
    }

    #[tokio::test]
    async fn provider_accounts_member_forbidden() {
        let state = state_with_stub(Stub::Valid).await;
        let created = create_account(&state, "perso-max").await;
        let member = bearer(&state, false).await;

        let (status, body, _) =
            call(&state, "GET", "/api/v1/provider-accounts", &member, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["code"], "FORBIDDEN");

        let (status, _, _) = call(
            &state,
            "DELETE",
            "/api/v1/provider-accounts/perso-max",
            &member,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Nothing was deleted.
        let (status, _, _) = call(
            &state,
            "GET",
            &format!(
                "/api/v1/provider-accounts/{}",
                created["data"]["id"].as_str().unwrap()
            ),
            &bearer(&state, true).await,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}
