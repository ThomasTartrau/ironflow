//! `POST /api/v1/provider-accounts` -- Add a Provider Account (admin only).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;

use crate::entities::{CreateProviderAccountRequest, ProviderAccountResponse};
use crate::error::ApiError;
use crate::provider_accounts::{self, AccountScope};
use crate::response::ok;
use crate::state::AppState;

/// Add a Provider Account. Admin only.
///
/// The token is checked against the provider before anything is stored: a
/// malformed or rejected token gets 422 and leaves nothing behind. A token
/// answered with 429 is stored and shown as limited until its window resets.
/// The response never includes the token.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/provider-accounts",
        tags = ["provider-accounts"],
        request_body(content = CreateProviderAccountRequest, description = "Account and its credential"),
        responses(
            (status = 201, description = "Provider account created", body = ProviderAccountResponse),
            (status = 400, description = "Invalid input or unknown kind"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 409, description = "Name already taken"),
            (status = 422, description = "Credential rejected"),
            (status = 502, description = "Provider unreachable")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn create_provider_account(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<CreateProviderAccountRequest>,
) -> Result<impl IntoResponse, ApiError> {
    provider_accounts::authorize(&auth, AccountScope::Manage)?;
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let account = provider_accounts::create(&state, auth.user_id, req).await?;
    let response = provider_accounts::to_response(state.store.as_ref(), account).await?;
    Ok((StatusCode::CREATED, ok(response)))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use ironflow_store::entities::{AuditLogFilter, EventKind};
    use serde_json::{Value, json};

    use crate::routes::provider_accounts::test_support::{
        Stub, TOKEN, bearer, call, create_account, state_with_api, state_with_stub,
    };

    fn body(name: &str, token: &str) -> Value {
        json!({"name": name, "kind": "claude_subscription", "token": token})
    }

    #[tokio::test]
    async fn provider_accounts_create_as_admin_returns_201_without_token() {
        let state = state_with_stub(Stub::Valid).await;
        let auth = bearer(&state, true);
        let (status, resp, text) = call(
            &state,
            "POST",
            "/api/v1/provider-accounts",
            &auth,
            Some(body("perso-max", TOKEN)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{text}");
        assert_eq!(resp["data"]["name"], "perso-max");
        assert_eq!(resp["data"]["display_name"], "perso-max");
        assert_eq!(resp["data"]["priority"], 100);
        assert_eq!(resp["data"]["state"], "ok");
        assert!(!text.contains(TOKEN));
        assert!(!text.contains("secret_key"));

        let audit = state
            .store
            .list_audit_logs(AuditLogFilter::default(), 1, 50)
            .await
            .unwrap();
        let entry = audit
            .items
            .iter()
            .find(|e| e.event_type == EventKind::ProviderAccountUpdated)
            .expect("audit entry");
        assert_eq!(entry.payload["change"], "created");
        assert!(!entry.payload.to_string().contains(TOKEN));
    }

    #[tokio::test]
    async fn provider_accounts_create_malformed_token_returns_422() {
        // Unreachable provider: a malformed token must fail before any call.
        let state = state_with_api("http://127.0.0.1:1");
        let auth = bearer(&state, true);
        let (status, resp, _) = call(
            &state,
            "POST",
            "/api/v1/provider-accounts",
            &auth,
            Some(body("bad", "sk-ant-oat01-invalid")),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(resp["error"]["code"], "ACCOUNT_CREDENTIAL_REJECTED");
    }

    #[tokio::test]
    async fn provider_accounts_create_unauthorized_token_returns_422_and_stores_nothing() {
        let state = state_with_stub(Stub::Unauthorized).await;
        let auth = bearer(&state, true);
        let (status, resp, text) = call(
            &state,
            "POST",
            "/api/v1/provider-accounts",
            &auth,
            Some(body("perso", TOKEN)),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(resp["error"]["message"].as_str().unwrap().contains("401"));
        assert!(!text.contains(TOKEN));
        assert!(
            state
                .store
                .find_provider_account_by_name("perso")
                .await
                .unwrap()
                .is_none()
        );
        let keys = state.store.list_secret_keys("accounts/").await.unwrap();
        assert!(keys.is_empty(), "no credential may be stored: {keys:?}");
    }

    #[tokio::test]
    async fn provider_accounts_create_unreachable_provider_returns_502() {
        let state = state_with_api("http://127.0.0.1:1");
        let auth = bearer(&state, true);
        let (status, _, _) = call(
            &state,
            "POST",
            "/api/v1/provider-accounts",
            &auth,
            Some(body("perso", TOKEN)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn provider_accounts_create_limited_account_is_stored_limited() {
        let state = state_with_stub(Stub::Limited).await;
        let resp = create_account(&state, "team-pro").await;
        assert_eq!(resp["data"]["state"], "limited");
        assert_eq!(resp["data"]["windows"][0]["window"], "five_hour");
        assert_eq!(resp["data"]["windows"][0]["status"], "rejected");
    }

    #[tokio::test]
    async fn provider_accounts_create_duplicate_name_returns_409() {
        let state = state_with_stub(Stub::Valid).await;
        create_account(&state, "perso-max").await;
        let (status, resp, _) = call(
            &state,
            "POST",
            "/api/v1/provider-accounts",
            &bearer(&state, true),
            Some(body("perso-max", TOKEN)),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(resp["error"]["code"], "CONFLICT");
    }

    #[tokio::test]
    async fn provider_accounts_create_rejects_unknown_kind_and_bad_name() {
        let state = state_with_stub(Stub::Valid).await;
        let auth = bearer(&state, true);
        let (status, _, _) = call(
            &state,
            "POST",
            "/api/v1/provider-accounts",
            &auth,
            Some(json!({"name": "perso", "kind": "nope", "token": TOKEN})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _, _) = call(
            &state,
            "POST",
            "/api/v1/provider-accounts",
            &auth,
            Some(body("Not A Slug", TOKEN)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
