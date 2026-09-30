//! Provider Account business logic, shared by the `/provider-accounts` routes.
//!
//! Routes stay thin: they authorize, parse, and call into this module. The
//! credential is written to the system secret `accounts/<id>/credential` and
//! never leaves it: no response, log, audit payload or event carries it.

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;
use tracing::{error, warn};
use uuid::Uuid;

use ironflow_auth::extractor::{AuthMethod, Authenticated};
use ironflow_core::account::{AccountError, AccountKind, AccountWindow, CredentialCheck};
use ironflow_engine::accounts::{window_from_store, window_to_store};
use ironflow_engine::notify::{
    Event, ProviderAccountChange, ProviderAccountUpdatedEvent, ProviderAccountUsageUpdatedEvent,
};
use ironflow_store::entities::{
    ApiKeyScope, EventKind, NewAuditLogEntry, NewProviderAccount, NewProviderAccountObservation,
    ProviderAccount, ProviderAccountUpdate, ProviderAccountWindow, provider_account_secret_key,
};
use ironflow_store::store::Store;

use crate::entities::{
    AccountState, AccountTestResult, CreateProviderAccountRequest, ProviderAccountResponse,
    ProviderAccountTestResponse, UpdateProviderAccountRequest,
};
use crate::error::ApiError;
use crate::state::AppState;

/// Default lifetime of a credential when the request gives no expiry.
const DEFAULT_CREDENTIAL_DAYS: i64 = 365;

/// Access level a route requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountScope {
    /// Read accounts and their usage.
    Read,
    /// Create, update, delete and test accounts.
    Manage,
}

/// Check that the caller may use the Provider Account routes.
///
/// Admin only. An API key must also carry `accounts_read`, `accounts_manage`
/// or `admin`, depending on `scope`.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] for a non-admin caller and
/// [`ApiError::InsufficientScope`] for an admin API key missing the scope.
pub fn authorize(auth: &Authenticated, scope: AccountScope) -> Result<(), ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }
    if let AuthMethod::ApiKey { scopes, .. } = &auth.method {
        let required = match scope {
            AccountScope::Read => ApiKeyScope::AccountsRead,
            AccountScope::Manage => ApiKeyScope::AccountsManage,
        };
        // Manage implies read.
        let permitted = ApiKeyScope::has_permission(scopes, &required)
            || (scope == AccountScope::Read
                && ApiKeyScope::has_permission(scopes, &ApiKeyScope::AccountsManage));
        if !permitted {
            return Err(ApiError::InsufficientScope);
        }
    }
    Ok(())
}

/// Find an account by UUID or by name.
///
/// # Errors
///
/// Returns [`ApiError::ProviderAccountNotFound`] when neither matches.
pub async fn resolve(store: &dyn Store, id_or_name: &str) -> Result<ProviderAccount, ApiError> {
    let by_id = match Uuid::parse_str(id_or_name) {
        Ok(id) => store.get_provider_account(id).await?,
        Err(_) => None,
    };
    let found = match by_id {
        Some(account) => Some(account),
        None => store.find_provider_account_by_name(id_or_name).await?,
    };
    found.ok_or_else(|| ApiError::ProviderAccountNotFound(id_or_name.to_string()))
}

/// Derive the display state of an account.
///
/// Only unscoped windows count: a model-scoped window blocks one model family,
/// not the account.
pub fn account_state(
    account: &ProviderAccount,
    windows: &[ProviderAccountWindow],
    now: DateTime<Utc>,
) -> AccountState {
    if account.auth_failed_at.is_some() {
        return AccountState::TokenInvalid;
    }
    if windows.is_empty() {
        return AccountState::NeverUsed;
    }
    let unscoped: Vec<AccountWindow> = windows
        .iter()
        .filter(|w| w.model_scope.is_none())
        .map(window_from_store)
        .collect();
    if unscoped.iter().any(|w| w.is_exhausted(now)) {
        return AccountState::Limited;
    }
    let max_utilization = unscoped
        .iter()
        .map(|w| w.effective_utilization(now))
        .fold(0.0, f64::max);
    if max_utilization >= account.alert_threshold {
        return AccountState::NearLimit;
    }
    AccountState::Ok
}

/// Build the response of one account, reading its windows.
///
/// # Errors
///
/// Returns [`ApiError::Store`] on storage failure.
pub async fn to_response(
    store: &dyn Store,
    account: ProviderAccount,
) -> Result<ProviderAccountResponse, ApiError> {
    let windows = store
        .list_provider_account_windows(vec![account.id])
        .await?;
    let state = account_state(&account, &windows, Utc::now());
    Ok(ProviderAccountResponse::new(account, windows, state))
}

fn kind_of(state: &AppState, kind: &str) -> Result<Arc<dyn AccountKind>, ApiError> {
    state
        .account_kinds
        .get(kind)
        .cloned()
        .ok_or_else(|| ApiError::BadRequest(format!("unknown account kind '{kind}'")))
}

/// Validate the format, then check the credential against the provider.
async fn check_token(kind: &dyn AccountKind, token: &str) -> Result<CredentialCheck, ApiError> {
    kind.validate_credential(token)
        .map_err(|e| ApiError::AccountCredentialRejected(e.to_string()))?;
    kind.check_credential(token).await.map_err(|e| match e {
        AccountError::InvalidCredential(message) => ApiError::AccountCredentialRejected(message),
        AccountError::Unauthorized { status } => ApiError::AccountCredentialRejected(format!(
            "token rejected by provider (HTTP {status})"
        )),
        AccountError::CheckFailed(message) => ApiError::BadGateway(message),
    })
}

async fn record_windows(
    state: &AppState,
    account: &ProviderAccount,
    windows: Vec<AccountWindow>,
    auth_failed: bool,
) -> Result<Vec<ProviderAccountWindow>, ApiError> {
    let recorded = state
        .store
        .record_provider_account_observation(
            account.id,
            NewProviderAccountObservation {
                windows: windows.into_iter().map(window_to_store).collect(),
                auth_failed,
            },
        )
        .await?;
    publish(
        state,
        Event::ProviderAccountUsageUpdated(ProviderAccountUsageUpdatedEvent {
            account_id: account.id,
            name: account.name.clone(),
            windows: recorded.clone(),
            at: Utc::now(),
        }),
    );
    Ok(recorded)
}

/// Push an event to the SSE clients.
fn publish(state: &AppState, event: Event) {
    // A send error only means no SSE client is listening.
    let _ = state.event_sender.send(event);
}

async fn audit_and_publish(
    state: &AppState,
    user_id: Uuid,
    account: &ProviderAccount,
    change: ProviderAccountChange,
    fields: Option<Vec<&'static str>>,
) -> Result<(), ApiError> {
    // Identifiers only: the credential never reaches the audit log.
    let mut payload = json!({
        "change": change.as_str(),
        "account_id": account.id,
        "name": account.name,
        "kind": account.kind,
    });
    if let Some(fields) = fields {
        payload["fields"] = json!(fields);
    }
    state
        .store
        .append_audit_log(NewAuditLogEntry {
            event_type: EventKind::ProviderAccountUpdated,
            payload,
            run_id: None,
            step_id: None,
            user_id: Some(user_id),
        })
        .await?;
    publish(
        state,
        Event::ProviderAccountUpdated(ProviderAccountUpdatedEvent {
            account_id: account.id,
            name: account.name.clone(),
            change,
            at: Utc::now(),
        }),
    );
    Ok(())
}

/// Create an account after checking its credential against the provider.
///
/// The secret is written before the row and deleted again when the insert
/// fails, so neither is left behind.
///
/// # Errors
///
/// [`ApiError::BadRequest`] for invalid input or an unknown kind,
/// [`ApiError::AccountCredentialRejected`] for a malformed or rejected token,
/// [`ApiError::Conflict`] for a taken name, [`ApiError::BadGateway`] when the
/// provider cannot be reached.
pub async fn create(
    state: &AppState,
    user_id: Uuid,
    req: CreateProviderAccountRequest,
) -> Result<ProviderAccount, ApiError> {
    let kind = kind_of(state, &req.kind)?;
    let token = req.token.trim().to_string();
    kind.validate_credential(&token)
        .map_err(|e| ApiError::AccountCredentialRejected(e.to_string()))?;
    if state
        .store
        .find_provider_account_by_name(&req.name)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(format!(
            "provider account '{}' already exists",
            req.name
        )));
    }
    let check = check_token(kind.as_ref(), &token).await?;

    let id = Uuid::now_v7();
    let secret_key = provider_account_secret_key(id);
    state.store.set_secret(&secret_key, &token).await?;

    let created = state
        .store
        .create_provider_account(NewProviderAccount {
            id,
            display_name: req.display_name.unwrap_or_else(|| req.name.clone()),
            name: req.name,
            kind: kind.id().to_string(),
            secret_key: secret_key.clone(),
            enabled: req.enabled.unwrap_or(true),
            priority: req.priority.unwrap_or(100),
            tags: req.tags.unwrap_or_default(),
            max_concurrency: req.max_concurrency,
            alert_threshold: req.alert_threshold.unwrap_or(0.8),
            expires_at: req
                .expires_at
                .unwrap_or_else(|| Utc::now() + TimeDelta::days(DEFAULT_CREDENTIAL_DAYS)),
            plan: req.plan,
            created_by: Some(user_id),
        })
        .await;
    let account = match created {
        Ok(account) => account,
        Err(e) => {
            if let Err(delete_err) = state.store.delete_secret(&secret_key).await {
                error!(account_id = %id, error = %delete_err, "failed to delete orphan provider account credential");
            }
            return Err(e.into());
        }
    };

    let windows = check.windows().to_vec();
    if !windows.is_empty() {
        record_windows(state, &account, windows, false).await?;
    }
    audit_and_publish(
        state,
        user_id,
        &account,
        ProviderAccountChange::Created,
        None,
    )
    .await?;
    Ok(account)
}

/// Update an account; a new token is checked before it replaces the old one.
///
/// # Errors
///
/// Same as [`create`], plus [`ApiError::ProviderAccountNotFound`].
pub async fn update(
    state: &AppState,
    user_id: Uuid,
    account: ProviderAccount,
    req: UpdateProviderAccountRequest,
) -> Result<ProviderAccount, ApiError> {
    let fields = req.changed_fields();
    let mut update = ProviderAccountUpdate {
        display_name: req.display_name,
        enabled: req.enabled,
        priority: req.priority,
        tags: req.tags,
        max_concurrency: req.max_concurrency,
        alert_threshold: req.alert_threshold,
        expires_at: req.expires_at,
        plan: req.plan,
        auth_failed_at: None,
    };

    let mut check_windows = None;
    if let Some(token) = req.token {
        let kind = kind_of(state, &account.kind)?;
        let token = token.trim().to_string();
        let check = check_token(kind.as_ref(), &token).await?;
        state.store.set_secret(&account.secret_key, &token).await?;
        update.auth_failed_at = Some(None);
        if update.expires_at.is_none() {
            update.expires_at = Some(Utc::now() + TimeDelta::days(DEFAULT_CREDENTIAL_DAYS));
        }
        check_windows = Some(check.windows().to_vec());
    }

    let updated = state
        .store
        .update_provider_account(account.id, update)
        .await?;

    match check_windows {
        Some(windows) => {
            if !windows.is_empty() {
                record_windows(state, &updated, windows, false).await?;
            }
            audit_and_publish(
                state,
                user_id,
                &updated,
                ProviderAccountChange::TokenReplaced,
                Some(fields),
            )
            .await?;
        }
        None => {
            audit_and_publish(
                state,
                user_id,
                &updated,
                ProviderAccountChange::Updated,
                Some(fields),
            )
            .await?;
        }
    }
    Ok(updated)
}

/// Delete an account and its credential.
///
/// # Errors
///
/// Returns [`ApiError::Store`] when the row or the secret cannot be deleted.
pub async fn delete(
    state: &AppState,
    user_id: Uuid,
    account: ProviderAccount,
) -> Result<(), ApiError> {
    if !state.store.delete_provider_account(account.id).await? {
        return Err(ApiError::ProviderAccountNotFound(account.id.to_string()));
    }
    state.store.delete_secret(&account.secret_key).await?;
    audit_and_publish(
        state,
        user_id,
        &account,
        ProviderAccountChange::Deleted,
        None,
    )
    .await
}

/// Check the stored credential against the provider and record the result.
///
/// # Errors
///
/// [`ApiError::AccountCredentialRejected`] when no credential is stored,
/// [`ApiError::BadGateway`] when the provider cannot be reached.
pub async fn test(
    state: &AppState,
    account: &ProviderAccount,
) -> Result<ProviderAccountTestResponse, ApiError> {
    let kind = kind_of(state, &account.kind)?;
    let secret = state
        .store
        .get_secret(&account.secret_key)
        .await?
        .ok_or_else(|| ApiError::AccountCredentialRejected("no credential stored".to_string()))?;

    let (result, windows, auth_failed) = match kind.check_credential(&secret.value).await {
        Ok(CredentialCheck::Valid { windows }) => (AccountTestResult::Valid, windows, false),
        Ok(CredentialCheck::Limited { windows }) => (AccountTestResult::Limited, windows, false),
        Err(AccountError::Unauthorized { status }) => {
            warn!(account = %account.name, status, "provider rejected the stored credential");
            (AccountTestResult::Unauthorized, Vec::new(), true)
        }
        Err(AccountError::InvalidCredential(message)) => {
            return Err(ApiError::AccountCredentialRejected(message));
        }
        Err(AccountError::CheckFailed(message)) => return Err(ApiError::BadGateway(message)),
    };

    let recorded = if windows.is_empty() && !auth_failed {
        Vec::new()
    } else {
        record_windows(state, account, windows, auth_failed).await?
    };
    Ok(ProviderAccountTestResponse {
        result,
        windows: recorded.into_iter().map(Into::into).collect(),
    })
}

#[cfg(test)]
mod tests {
    use ironflow_store::entities::AccountWindowStatus;

    use super::*;

    fn account(threshold: f64) -> ProviderAccount {
        let now = Utc::now();
        ProviderAccount {
            id: Uuid::now_v7(),
            name: "perso".to_string(),
            display_name: "Perso".to_string(),
            kind: "claude_subscription".to_string(),
            secret_key: "accounts/x/credential".to_string(),
            enabled: true,
            priority: 100,
            tags: Vec::new(),
            max_concurrency: None,
            alert_threshold: threshold,
            expires_at: now + TimeDelta::days(1),
            plan: None,
            auth_failed_at: None,
            created_by: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn window(
        utilization: f64,
        status: AccountWindowStatus,
        scope: Option<&str>,
    ) -> ProviderAccountWindow {
        ProviderAccountWindow {
            account_id: Uuid::now_v7(),
            window: "five_hour".to_string(),
            utilization,
            resets_at: Some(Utc::now() + TimeDelta::hours(1)),
            status,
            model_scope: scope.map(str::to_string),
            observed_at: Utc::now(),
        }
    }

    #[test]
    fn account_state_covers_every_variant() {
        let now = Utc::now();
        let ok = account(0.8);
        assert_eq!(account_state(&ok, &[], now), AccountState::NeverUsed);
        assert_eq!(
            account_state(&ok, &[window(0.2, AccountWindowStatus::Allowed, None)], now),
            AccountState::Ok
        );
        assert_eq!(
            account_state(
                &ok,
                &[window(0.85, AccountWindowStatus::AllowedWarning, None)],
                now
            ),
            AccountState::NearLimit
        );
        assert_eq!(
            account_state(
                &ok,
                &[window(1.0, AccountWindowStatus::Rejected, None)],
                now
            ),
            AccountState::Limited
        );
        let mut failed = account(0.8);
        failed.auth_failed_at = Some(now);
        assert_eq!(account_state(&failed, &[], now), AccountState::TokenInvalid);
    }

    #[test]
    fn account_state_ignores_model_scoped_windows() {
        let now = Utc::now();
        let windows = [window(1.0, AccountWindowStatus::Rejected, Some("opus"))];
        assert_eq!(
            account_state(&account(0.8), &windows, now),
            AccountState::Ok
        );
    }

    #[test]
    fn authorize_rejects_members_and_unscoped_keys() {
        let member = Authenticated {
            user_id: Uuid::now_v7(),
            method: AuthMethod::Jwt {
                username: "bob".to_string(),
                is_admin: false,
            },
        };
        assert!(matches!(
            authorize(&member, AccountScope::Read),
            Err(ApiError::Forbidden)
        ));

        let key = |scopes: Vec<ApiKeyScope>| Authenticated {
            user_id: Uuid::now_v7(),
            method: AuthMethod::ApiKey {
                key_id: Uuid::now_v7(),
                key_name: "ci".to_string(),
                scopes,
                owner_is_admin: true,
            },
        };
        assert!(matches!(
            authorize(&key(vec![ApiKeyScope::RunsRead]), AccountScope::Read),
            Err(ApiError::InsufficientScope)
        ));
        assert!(authorize(&key(vec![ApiKeyScope::AccountsRead]), AccountScope::Read).is_ok());
        assert!(matches!(
            authorize(&key(vec![ApiKeyScope::AccountsRead]), AccountScope::Manage),
            Err(ApiError::InsufficientScope)
        ));
        assert!(authorize(&key(vec![ApiKeyScope::AccountsManage]), AccountScope::Read).is_ok());
        assert!(authorize(&key(vec![ApiKeyScope::Admin]), AccountScope::Manage).is_ok());
    }
}
