//! `PATCH /api/v1/provider-accounts/{id}` -- Update a Provider Account (admin only).

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;

#[cfg(feature = "openapi")]
use crate::entities::ProviderAccountResponse;
use crate::entities::UpdateProviderAccountRequest;
use crate::error::ApiError;
use crate::provider_accounts::{self, AccountScope};
use crate::response::ok;
use crate::state::AppState;

/// Update a Provider Account. Admin only.
///
/// `name` and `kind` are immutable. A new `token` is checked against the
/// provider before it replaces the stored one. The response never includes
/// the token.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        patch,
        path = "/api/v1/provider-accounts/{id}",
        tags = ["provider-accounts"],
        params(("id" = String, Path, description = "Account UUID or name")),
        request_body(content = UpdateProviderAccountRequest, description = "Fields to change"),
        responses(
            (status = 200, description = "Provider account updated", body = ProviderAccountResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Provider account not found"),
            (status = 422, description = "Credential rejected"),
            (status = 502, description = "Provider unreachable")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn update_provider_account(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProviderAccountRequest>,
) -> Result<impl IntoResponse, ApiError> {
    provider_accounts::authorize(&auth, AccountScope::Manage)?;
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    req.validate_nested().map_err(ApiError::BadRequest)?;

    let account = provider_accounts::resolve(state.store.as_ref(), &id).await?;
    let updated = provider_accounts::update(&state, auth.user_id, account, req).await?;
    let response = provider_accounts::to_response(state.store.as_ref(), updated).await?;
    Ok(ok(response))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use ironflow_store::entities::{AuditLogFilter, EventKind};
    use serde_json::json;

    use crate::routes::provider_accounts::test_support::{
        Stub, TOKEN, bearer, call, create_account, state_with_stub,
    };

    #[tokio::test]
    async fn provider_accounts_update_settings() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        let (status, resp, _) = call(
            &state,
            "PATCH",
            "/api/v1/provider-accounts/perso-max",
            &bearer(&state, true),
            Some(json!({"enabled": false, "max_concurrency": 2, "tags": ["team"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"]["enabled"], false);
        assert_eq!(resp["data"]["max_concurrency"], 2);
        assert_eq!(resp["data"]["tags"], json!(["team"]));

        let (status, resp, _) = call(
            &state,
            "PATCH",
            "/api/v1/provider-accounts/perso-max",
            &bearer(&state, true),
            Some(json!({"max_concurrency": null})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp["data"]["max_concurrency"].is_null());
    }

    #[tokio::test]
    async fn provider_accounts_update_rejects_name_change() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        let (status, _, _) = call(
            &state,
            "PATCH",
            "/api/v1/provider-accounts/perso-max",
            &bearer(&state, true),
            Some(json!({"name": "renamed"})),
        )
        .await;
        assert!(status.is_client_error());
        assert!(
            state
                .store
                .find_provider_account_by_name("perso-max")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn provider_accounts_update_replaces_token_and_audits() {
        let state = state_with_stub(Stub::Valid).await;
        let created = create_account(&state, "perso-max").await;
        let new_token = format!("{TOKEN}-rotated");
        let (status, resp, text) = call(
            &state,
            "PATCH",
            "/api/v1/provider-accounts/perso-max",
            &bearer(&state, true),
            Some(json!({"token": new_token})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(!text.contains(&new_token));
        assert!(resp["data"]["auth_failed_at"].is_null());

        let account = state
            .store
            .find_provider_account_by_name("perso-max")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            account.id.to_string(),
            created["data"]["id"].as_str().unwrap()
        );
        let secret = state
            .store
            .get_secret(&account.secret_key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(secret.value, new_token);

        let audit = state
            .store
            .list_audit_logs(AuditLogFilter::default(), 1, 50)
            .await
            .unwrap();
        let replaced = audit
            .items
            .iter()
            .find(|e| {
                e.event_type == EventKind::ProviderAccountUpdated
                    && e.payload["change"] == "token_replaced"
            })
            .expect("token_replaced audit entry");
        assert!(!replaced.payload.to_string().contains(&new_token));
    }

    #[tokio::test]
    async fn provider_accounts_update_with_malformed_token_keeps_old_one() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        let (status, _, _) = call(
            &state,
            "PATCH",
            "/api/v1/provider-accounts/perso-max",
            &bearer(&state, true),
            Some(json!({"token": "sk-ant-oat01-short"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let account = state
            .store
            .find_provider_account_by_name("perso-max")
            .await
            .unwrap()
            .unwrap();
        let secret = state
            .store
            .get_secret(&account.secret_key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(secret.value, TOKEN);
    }
}
