//! Provider Account entities: AI provider accounts, their usage windows and history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use uuid::Uuid;

/// Prefix of the system secrets holding Provider Account credentials.
///
/// Keys under this prefix are hidden from the Secrets listing and refused by
/// the Secrets API.
pub const PROVIDER_ACCOUNT_SECRET_PREFIX: &str = "accounts/";

/// Secret key holding the credential of the account `id`.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::provider_account_secret_key;
/// use uuid::Uuid;
///
/// let key = provider_account_secret_key(Uuid::nil());
/// assert_eq!(key, "accounts/00000000-0000-0000-0000-000000000000/credential");
/// ```
pub fn provider_account_secret_key(id: Uuid) -> String {
    format!("{PROVIDER_ACCOUNT_SECRET_PREFIX}{id}/credential")
}

/// Status of a usage window, as reported by the provider.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::AccountWindowStatus;
///
/// let status: AccountWindowStatus = "allowed_warning".parse().unwrap();
/// assert_eq!(status, AccountWindowStatus::AllowedWarning);
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AccountWindowStatus {
    /// Requests are allowed.
    Allowed,
    /// Requests are allowed, close to the limit.
    AllowedWarning,
    /// Requests are rejected until the window resets.
    Rejected,
}

/// A persisted Provider Account.
///
/// The credential itself lives in the secret `secret_key`, never here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAccount {
    /// Account ID (UUID v7).
    pub id: Uuid,
    /// Unique slug.
    pub name: String,
    /// Human-readable name.
    pub display_name: String,
    /// Account kind (e.g. `claude_subscription`).
    pub kind: String,
    /// Key of the secret holding the credential.
    pub secret_key: String,
    /// Whether the account may be selected.
    pub enabled: bool,
    /// Lower values are preferred by the `priority` strategy.
    pub priority: i32,
    /// Free-form tags.
    pub tags: Vec<String>,
    /// Maximum concurrent steps, `None` for unlimited.
    pub max_concurrency: Option<u32>,
    /// Utilization from which the account is shown as near its limit.
    pub alert_threshold: f64,
    /// When the credential expires.
    pub expires_at: DateTime<Utc>,
    /// Subscription plan (`pro`, `max`), informative.
    pub plan: Option<String>,
    /// When the provider last rejected the credential, `None` when valid.
    pub auth_failed_at: Option<DateTime<Utc>>,
    /// User who created the account.
    pub created_by: Option<Uuid>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Request to create a Provider Account.
///
/// The caller generates `id` so the secret key can be built first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProviderAccount {
    /// Account ID (UUID v7).
    pub id: Uuid,
    /// Unique slug.
    pub name: String,
    /// Human-readable name.
    pub display_name: String,
    /// Account kind.
    pub kind: String,
    /// Key of the secret holding the credential.
    pub secret_key: String,
    /// Whether the account may be selected.
    pub enabled: bool,
    /// Priority.
    pub priority: i32,
    /// Tags.
    pub tags: Vec<String>,
    /// Maximum concurrent steps.
    pub max_concurrency: Option<u32>,
    /// Alert threshold in `(0, 1]`.
    pub alert_threshold: f64,
    /// When the credential expires.
    pub expires_at: DateTime<Utc>,
    /// Subscription plan.
    pub plan: Option<String>,
    /// Creating user.
    pub created_by: Option<Uuid>,
}

/// Partial update of a Provider Account. `None` fields are left unchanged.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ProviderAccountUpdate;
///
/// let update = ProviderAccountUpdate {
///     enabled: Some(false),
///     max_concurrency: Some(None),
///     ..ProviderAccountUpdate::default()
/// };
/// assert_eq!(update.max_concurrency, Some(None));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderAccountUpdate {
    /// New display name.
    pub display_name: Option<String>,
    /// Enable or disable.
    pub enabled: Option<bool>,
    /// New priority.
    pub priority: Option<i32>,
    /// New tags (replaces).
    pub tags: Option<Vec<String>>,
    /// New max concurrency; `Some(None)` clears it.
    pub max_concurrency: Option<Option<u32>>,
    /// New alert threshold.
    pub alert_threshold: Option<f64>,
    /// New expiry.
    pub expires_at: Option<DateTime<Utc>>,
    /// New plan; `Some(None)` clears it.
    pub plan: Option<Option<String>>,
    /// New auth failure mark; `Some(None)` clears it.
    pub auth_failed_at: Option<Option<DateTime<Utc>>>,
}

/// Latest reading of one usage window of an account.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAccountWindow {
    /// Account the window belongs to.
    pub account_id: Uuid,
    /// Window name (`five_hour`, `seven_day`).
    pub window: String,
    /// Fraction used, `0.0..=1.0`.
    pub utilization: f64,
    /// When the window resets.
    pub resets_at: Option<DateTime<Utc>>,
    /// Provider status.
    pub status: AccountWindowStatus,
    /// Model family the window applies to, `None` for every model.
    pub model_scope: Option<String>,
    /// When the window was observed.
    pub observed_at: DateTime<Utc>,
}

/// One historical observation of a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAccountUsagePoint {
    /// Observation ID.
    pub id: Uuid,
    /// Account the observation belongs to.
    pub account_id: Uuid,
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

/// A window observed during an invocation or a credential check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewAccountWindow {
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

/// Observations to record for an account.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::NewProviderAccountObservation;
///
/// let observation = NewProviderAccountObservation { windows: Vec::new(), auth_failed: true };
/// assert!(observation.auth_failed);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NewProviderAccountObservation {
    /// Observed windows.
    pub windows: Vec<NewAccountWindow>,
    /// Whether the provider rejected the credential.
    #[serde(default)]
    pub auth_failed: bool,
}

/// An account eligible for selection, with its windows and load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAccountCandidate {
    /// The account.
    pub account: ProviderAccount,
    /// Its latest windows.
    pub windows: Vec<ProviderAccountWindow>,
    /// Steps currently running under it.
    pub running_steps: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_key_is_under_the_accounts_prefix() {
        let id = Uuid::now_v7();
        let key = provider_account_secret_key(id);
        assert!(key.starts_with(PROVIDER_ACCOUNT_SECRET_PREFIX));
        assert!(key.ends_with("/credential"));
        assert!(key.contains(&id.to_string()));
    }

    #[test]
    fn window_status_wire_format() {
        assert_eq!(AccountWindowStatus::Rejected.to_string(), "rejected");
        assert_eq!(
            serde_json::to_string(&AccountWindowStatus::AllowedWarning).unwrap(),
            "\"allowed_warning\""
        );
        assert!("nope".parse::<AccountWindowStatus>().is_err());
    }

    #[test]
    fn observation_auth_failed_defaults_to_false() {
        let observation: NewProviderAccountObservation =
            serde_json::from_str(r#"{"windows":[]}"#).unwrap();
        assert!(!observation.auth_failed);
    }
}
