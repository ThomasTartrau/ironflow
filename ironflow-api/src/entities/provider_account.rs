//! Provider Account request and response DTOs.
//!
//! No response carries the credential or the key of the secret holding it.
//! The request DTOs redact the token in their `Debug` output.

use std::fmt;

use chrono::{DateTime, Utc};
use ironflow_core::account::AccountFormField;
use ironflow_store::entities::{
    AccountWindowStatus, ProviderAccount, ProviderAccountUsagePoint, ProviderAccountWindow,
};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;
use validator::{Validate, ValidationError};

/// Maximum length of an account name or tag.
const MAX_SLUG_LEN: usize = 63;

/// Check a slug: `^[a-z0-9][a-z0-9-]{0,62}$`.
fn is_slug(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.len() <= MAX_SLUG_LEN
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn validate_slug(value: &str) -> Result<(), ValidationError> {
    if is_slug(value) {
        return Ok(());
    }
    let mut err = ValidationError::new("invalid_slug");
    err.message = Some("must match ^[a-z0-9][a-z0-9-]{0,62}$".into());
    Err(err)
}

fn validate_tags(tags: &[String]) -> Result<(), ValidationError> {
    if tags.iter().all(|t| is_slug(t)) {
        return Ok(());
    }
    let mut err = ValidationError::new("invalid_tag");
    err.message = Some("every tag must match ^[a-z0-9][a-z0-9-]{0,62}$".into());
    Err(err)
}

fn validate_threshold(threshold: f64) -> Result<(), ValidationError> {
    if threshold > 0.0 && threshold <= 1.0 {
        return Ok(());
    }
    let mut err = ValidationError::new("invalid_threshold");
    err.message = Some("alert_threshold must be in (0, 1]".into());
    Err(err)
}

/// Deserialize a present field (`null` included) as `Some`, so a missing
/// field stays `None` and `null` becomes `Some(None)`.
fn deserialize_some<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Request body to add a Provider Account. Admin only.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Deserialize, Validate)]
pub struct CreateProviderAccountRequest {
    /// Unique slug, immutable (`^[a-z0-9][a-z0-9-]{0,62}$`).
    #[validate(custom(function = "validate_slug"))]
    pub name: String,
    /// Human-readable name, defaults to `name`.
    #[validate(length(min = 1, max = 200))]
    pub display_name: Option<String>,
    /// Account kind (see `GET /provider-accounts/kinds`).
    #[validate(length(min = 1, max = 64))]
    pub kind: String,
    /// Credential, write-only (e.g. a `claude setup-token` token).
    #[validate(length(min = 1, max = 4096))]
    pub token: String,
    /// Whether the account may be selected (default `true`).
    pub enabled: Option<bool>,
    /// Priority, lower is preferred (default 100).
    pub priority: Option<i32>,
    /// Tags, each a slug.
    #[validate(custom(function = "validate_tags"))]
    pub tags: Option<Vec<String>>,
    /// Maximum concurrent steps (>= 1).
    #[validate(range(min = 1))]
    pub max_concurrency: Option<u32>,
    /// Utilization from which the account is shown as near its limit, in `(0, 1]` (default 0.8).
    #[validate(custom(function = "validate_threshold"))]
    pub alert_threshold: Option<f64>,
    /// When the credential expires (default: now + 365 days).
    pub expires_at: Option<DateTime<Utc>>,
    /// Subscription plan, informative (`pro`, `max`).
    #[validate(length(max = 64))]
    pub plan: Option<String>,
}

impl fmt::Debug for CreateProviderAccountRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateProviderAccountRequest")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Request body to update a Provider Account. All fields optional; `name`
/// and `kind` are immutable.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Default, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct UpdateProviderAccountRequest {
    /// New display name.
    #[validate(length(min = 1, max = 200))]
    pub display_name: Option<String>,
    /// Enable or disable.
    pub enabled: Option<bool>,
    /// New priority.
    pub priority: Option<i32>,
    /// New tags (replaces the list).
    #[validate(custom(function = "validate_tags"))]
    pub tags: Option<Vec<String>>,
    /// New max concurrency; `null` removes the limit.
    #[serde(default, deserialize_with = "deserialize_some")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<u32>))]
    pub max_concurrency: Option<Option<u32>>,
    /// New alert threshold in `(0, 1]`.
    #[validate(custom(function = "validate_threshold"))]
    pub alert_threshold: Option<f64>,
    /// New expiry of the credential.
    pub expires_at: Option<DateTime<Utc>>,
    /// New plan; `null` clears it.
    #[serde(default, deserialize_with = "deserialize_some")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub plan: Option<Option<String>>,
    /// Replacement credential, write-only. Checked against the provider.
    #[validate(length(min = 1, max = 4096))]
    pub token: Option<String>,
}

impl UpdateProviderAccountRequest {
    /// Check the fields `validator` cannot express on nested options.
    ///
    /// # Errors
    ///
    /// Returns a message when `max_concurrency` is `0` or `plan` is too long.
    pub fn validate_nested(&self) -> Result<(), String> {
        if self.max_concurrency == Some(Some(0)) {
            return Err("max_concurrency must be >= 1".to_string());
        }
        if let Some(Some(plan)) = &self.plan
            && plan.len() > 64
        {
            return Err("plan must be at most 64 characters".to_string());
        }
        Ok(())
    }

    /// Names of the fields present in the request, except the token.
    pub fn changed_fields(&self) -> Vec<&'static str> {
        [
            ("display_name", self.display_name.is_some()),
            ("enabled", self.enabled.is_some()),
            ("priority", self.priority.is_some()),
            ("tags", self.tags.is_some()),
            ("max_concurrency", self.max_concurrency.is_some()),
            ("alert_threshold", self.alert_threshold.is_some()),
            ("expires_at", self.expires_at.is_some()),
            ("plan", self.plan.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, present)| present.then_some(name))
        .collect()
    }
}

impl fmt::Debug for UpdateProviderAccountRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdateProviderAccountRequest")
            .field("fields", &self.changed_fields())
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Derived state of an account, for display.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    /// Usable, below its alert threshold.
    Ok,
    /// A window is at or above the alert threshold.
    NearLimit,
    /// A window rejects requests until it resets.
    Limited,
    /// The provider rejected the credential.
    TokenInvalid,
    /// No window observed yet.
    NeverUsed,
}

/// One usage window of an account.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountWindowResponse {
    /// Window name (`five_hour`, `seven_day`).
    pub window: String,
    /// Fraction used, `0.0..=1.0`.
    pub utilization: f64,
    /// When the window resets.
    pub resets_at: Option<DateTime<Utc>>,
    /// Provider status.
    pub status: AccountWindowStatus,
    /// Model family the window applies to, `null` for every model.
    pub model_scope: Option<String>,
    /// When the window was observed.
    pub observed_at: DateTime<Utc>,
}

impl From<ProviderAccountWindow> for AccountWindowResponse {
    fn from(w: ProviderAccountWindow) -> Self {
        Self {
            window: w.window,
            utilization: w.utilization,
            resets_at: w.resets_at,
            status: w.status,
            model_scope: w.model_scope,
            observed_at: w.observed_at,
        }
    }
}

/// One historical observation of a window.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsagePointResponse {
    /// Window name.
    pub window: String,
    /// Fraction used.
    pub utilization: f64,
    /// When the window resets.
    pub resets_at: Option<DateTime<Utc>>,
    /// Provider status.
    pub status: AccountWindowStatus,
    /// Model scope.
    pub model_scope: Option<String>,
    /// When the window was observed.
    pub observed_at: DateTime<Utc>,
}

impl From<ProviderAccountUsagePoint> for AccountUsagePointResponse {
    fn from(p: ProviderAccountUsagePoint) -> Self {
        Self {
            window: p.window,
            utilization: p.utilization,
            resets_at: p.resets_at,
            status: p.status,
            model_scope: p.model_scope,
            observed_at: p.observed_at,
        }
    }
}

/// A Provider Account, without its credential.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAccountResponse {
    /// Account ID.
    pub id: Uuid,
    /// Unique slug.
    pub name: String,
    /// Human-readable name.
    pub display_name: String,
    /// Account kind.
    pub kind: String,
    /// Whether the account may be selected.
    pub enabled: bool,
    /// Priority, lower is preferred.
    pub priority: i32,
    /// Tags.
    pub tags: Vec<String>,
    /// Maximum concurrent steps, `null` for unlimited.
    pub max_concurrency: Option<u32>,
    /// Alert threshold.
    pub alert_threshold: f64,
    /// When the credential expires.
    pub expires_at: DateTime<Utc>,
    /// Subscription plan.
    pub plan: Option<String>,
    /// Creating user.
    pub created_by: Option<Uuid>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// When the provider last rejected the credential.
    pub auth_failed_at: Option<DateTime<Utc>>,
    /// Derived state.
    pub state: AccountState,
    /// Latest windows.
    pub windows: Vec<AccountWindowResponse>,
}

impl ProviderAccountResponse {
    /// Build the response from an account, its windows and its derived state.
    pub fn new(
        account: ProviderAccount,
        windows: Vec<ProviderAccountWindow>,
        state: AccountState,
    ) -> Self {
        Self {
            id: account.id,
            name: account.name,
            display_name: account.display_name,
            kind: account.kind,
            enabled: account.enabled,
            priority: account.priority,
            tags: account.tags,
            max_concurrency: account.max_concurrency,
            alert_threshold: account.alert_threshold,
            expires_at: account.expires_at,
            plan: account.plan,
            created_by: account.created_by,
            created_at: account.created_at,
            updated_at: account.updated_at,
            auth_failed_at: account.auth_failed_at,
            state,
            windows: windows
                .into_iter()
                .map(AccountWindowResponse::from)
                .collect(),
        }
    }
}

/// Current windows and history of an account.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAccountUsageResponse {
    /// Account ID.
    pub account_id: Uuid,
    /// Account name.
    pub name: String,
    /// Latest windows.
    pub windows: Vec<AccountWindowResponse>,
    /// Observations over the requested period, oldest first.
    pub history: Vec<AccountUsagePointResponse>,
}

/// Outcome of a live credential test.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountTestResult {
    /// The provider accepted the credential.
    Valid,
    /// The credential is valid but rate limited.
    Limited,
    /// The provider rejected the credential.
    Unauthorized,
}

/// Response of `POST /provider-accounts/{id}/test`.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAccountTestResponse {
    /// Outcome.
    pub result: AccountTestResult,
    /// Windows reported by the provider.
    pub windows: Vec<AccountWindowResponse>,
}

/// A form field of an account kind.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountFormFieldResponse {
    /// Field identifier.
    pub name: String,
    /// Label.
    pub label: String,
    /// Whether the field is a secret (write-only).
    pub secret: bool,
    /// Help text.
    pub help: String,
}

impl From<AccountFormField> for AccountFormFieldResponse {
    fn from(field: AccountFormField) -> Self {
        Self {
            name: field.name.to_string(),
            label: field.label.to_string(),
            secret: field.secret,
            help: field.help.to_string(),
        }
    }
}

/// A supported account kind.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountKindResponse {
    /// Kind identifier.
    pub id: String,
    /// Human-readable name.
    pub display_name: String,
    /// Fields of the add form.
    pub fields: Vec<AccountFormFieldResponse>,
}

/// Query of `GET /provider-accounts`.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[derive(Debug, Default, Deserialize)]
pub struct ListProviderAccountsQuery {
    /// Only accounts of this kind.
    pub kind: Option<String>,
    /// Page number (1-based).
    pub page: Option<u32>,
    /// Items per page (max 100).
    pub per_page: Option<u32>,
}

/// Query of `GET /provider-accounts/{id}/usage`.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[derive(Debug, Default, Deserialize)]
pub struct UsageQuery {
    /// History depth in days, 1 to 90 (default 30).
    pub days: Option<u32>,
}

impl UsageQuery {
    /// Requested depth, defaulted and clamped to `1..=90`.
    pub fn days(&self) -> u32 {
        self.days.unwrap_or(30).clamp(1, 90)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_str, json};

    use super::*;

    const TOKEN: &str = "sk-ant-oat01-secret-token-value";

    fn create_request(name: &str) -> CreateProviderAccountRequest {
        from_str(&json!({"name": name, "kind": "claude_subscription", "token": TOKEN}).to_string())
            .unwrap()
    }

    #[test]
    fn slug_validation() {
        assert!(is_slug("perso-max"));
        assert!(is_slug("0"));
        assert!(!is_slug(""));
        assert!(!is_slug("-perso"));
        assert!(!is_slug("Perso"));
        assert!(!is_slug("perso_max"));
        assert!(!is_slug(&"a".repeat(64)));
        assert!(is_slug(&"a".repeat(63)));
    }

    #[test]
    fn create_request_validates_name_and_tags() {
        assert!(create_request("perso").validate().is_ok());
        assert!(create_request("Bad Name").validate().is_err());
        let mut req = create_request("perso");
        req.tags = Some(vec!["ok".to_string(), "Not OK".to_string()]);
        assert!(req.validate().is_err());
        let mut req = create_request("perso");
        req.alert_threshold = Some(0.0);
        assert!(req.validate().is_err());
        let mut req = create_request("perso");
        req.max_concurrency = Some(0);
        assert!(req.validate().is_err());
    }

    #[test]
    fn request_debug_redacts_token() {
        let req = create_request("perso");
        assert!(!format!("{req:?}").contains(TOKEN));
        let update = UpdateProviderAccountRequest {
            token: Some(TOKEN.to_string()),
            ..UpdateProviderAccountRequest::default()
        };
        assert!(!format!("{update:?}").contains(TOKEN));
    }

    #[test]
    fn update_request_distinguishes_null_from_missing() {
        let update: UpdateProviderAccountRequest =
            from_str(r#"{"max_concurrency": null, "priority": 5}"#).unwrap();
        assert_eq!(update.max_concurrency, Some(None));
        assert_eq!(update.plan, None);
        assert_eq!(update.changed_fields(), vec!["priority", "max_concurrency"]);

        let zero: UpdateProviderAccountRequest = from_str(r#"{"max_concurrency": 0}"#).unwrap();
        assert!(zero.validate_nested().is_err());
    }

    #[test]
    fn update_request_rejects_name_change() {
        let err = from_str::<UpdateProviderAccountRequest>(r#"{"name": "other"}"#);
        assert!(err.is_err());
    }

    #[test]
    fn usage_query_days_are_clamped() {
        assert_eq!(UsageQuery::default().days(), 30);
        assert_eq!(UsageQuery { days: Some(0) }.days(), 1);
        assert_eq!(UsageQuery { days: Some(365) }.days(), 90);
    }
}
