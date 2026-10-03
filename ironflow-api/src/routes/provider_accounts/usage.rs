//! `GET /api/v1/provider-accounts/{id}/usage` -- Windows and history (admin only).

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::{TimeDelta, Utc};

use ironflow_auth::extractor::Authenticated;

use crate::entities::{ProviderAccountUsageResponse, UsageQuery};
use crate::error::ApiError;
use crate::provider_accounts::{self, AccountScope};
use crate::response::ok;
use crate::state::AppState;

/// Current windows of a Provider Account and their history over `days`
/// (default 30, at most 90). Admin only.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/provider-accounts/{id}/usage",
        tags = ["provider-accounts"],
        params(("id" = String, Path, description = "Account UUID or name"), UsageQuery),
        responses(
            (status = 200, description = "Usage of the account", body = ProviderAccountUsageResponse),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Provider account not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn provider_account_usage(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<UsageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    provider_accounts::authorize(&auth, AccountScope::Read)?;
    let account = provider_accounts::resolve(state.store.as_ref(), &id).await?;
    let since = Utc::now() - TimeDelta::days(i64::from(query.days()));
    let windows = state
        .store
        .list_provider_account_windows(vec![account.id])
        .await?;
    let history = state
        .store
        .list_provider_account_usage(account.id, since)
        .await?;
    Ok(ok(ProviderAccountUsageResponse {
        account_id: account.id,
        name: account.name,
        windows: windows.into_iter().map(Into::into).collect(),
        history: history.into_iter().map(Into::into).collect(),
    }))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::routes::provider_accounts::test_support::{
        Stub, bearer, call, create_account, state_with_stub,
    };

    #[tokio::test]
    async fn provider_accounts_usage_returns_windows() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        let (status, resp, _) = call(
            &state,
            "GET",
            "/api/v1/provider-accounts/perso-max/usage?days=7",
            &bearer(&state, true).await,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<&str> = resp["data"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["window"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"five_hour"));
        assert!(names.contains(&"seven_day"));
        assert_eq!(resp["data"]["history"].as_array().unwrap().len(), 2);
        assert_eq!(resp["data"]["name"], "perso-max");
    }
}
