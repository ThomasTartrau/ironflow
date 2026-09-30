//! Provider Accounts: credentials and usage limits of AI provider accounts.
//!
//! A Provider Account is an account at an AI provider (v1: a Claude Pro/Max
//! subscription). This module holds the provider-agnostic pieces:
//!
//! * [`AccountWindow`] - one usage-limit window (e.g. the 5 hour window) as
//!   last observed.
//! * [`AccountCredential`] / [`AccountSession`] - the credential a provider
//!   injects into its process, and the recorder it reports observed windows to.
//! * [`AccountKind`] - a kind of account (form fields, credential validation,
//!   live credential check). [`ClaudeSubscriptionKind`] is the v1 kind.
//!
//! Selection among accounts lives in [`crate::account_strategy`].
//!
//! # Examples
//!
//! ```
//! use ironflow_core::account::{AccountKind, ClaudeSubscriptionKind};
//!
//! let kind = ClaudeSubscriptionKind::new();
//! assert!(kind.validate_credential("not-a-token").is_err());
//! ```

use std::fmt;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use strum::{Display, EnumString};
use thiserror::Error;

/// Status of a usage-limit window as reported by the provider.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::WindowStatus;
///
/// assert_eq!(WindowStatus::AllowedWarning.to_string(), "allowed_warning");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WindowStatus {
    /// Requests are allowed.
    Allowed,
    /// Requests are allowed, but the window is close to its limit.
    AllowedWarning,
    /// Requests are rejected until the window resets.
    Rejected,
}

/// One usage-limit window of a Provider Account, as last observed.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_core::account::{AccountWindow, WindowStatus};
///
/// let window = AccountWindow {
///     window: "seven_day".to_string(),
///     utilization: 1.0,
///     resets_at: None,
///     status: WindowStatus::Rejected,
///     model_scope: Some("opus".to_string()),
///     observed_at: Utc::now(),
/// };
/// assert!(window.applies_to("claude-opus-4-1"));
/// assert!(!window.applies_to("claude-sonnet-4-5"));
/// assert!(window.is_exhausted(Utc::now()));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountWindow {
    /// Window name, e.g. `"five_hour"` or `"seven_day"`.
    pub window: String,
    /// Fraction of the window used, between `0.0` and `1.0`.
    pub utilization: f64,
    /// When the window resets, if known.
    pub resets_at: Option<DateTime<Utc>>,
    /// Provider-reported status.
    pub status: WindowStatus,
    /// Model family the window applies to (`"opus"`), `None` for every model.
    pub model_scope: Option<String>,
    /// When the window was observed.
    pub observed_at: DateTime<Utc>,
}

impl AccountWindow {
    /// Whether this window constrains requests for `model`.
    ///
    /// # Examples
    ///
    /// See [`AccountWindow`].
    pub fn applies_to(&self, model: &str) -> bool {
        match &self.model_scope {
            None => true,
            Some(scope) => model
                .to_ascii_lowercase()
                .contains(&scope.to_ascii_lowercase()),
        }
    }

    /// Whether the window rejects requests at `now`.
    ///
    /// A rejected window stops blocking once its `resets_at` has passed.
    ///
    /// # Examples
    ///
    /// See [`AccountWindow`].
    pub fn is_exhausted(&self, now: DateTime<Utc>) -> bool {
        self.status == WindowStatus::Rejected && self.resets_at.is_none_or(|reset| reset > now)
    }

    /// Utilization at `now`: `0.0` once the window has reset.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{TimeDelta, Utc};
    /// use ironflow_core::account::{AccountWindow, WindowStatus};
    ///
    /// let now = Utc::now();
    /// let window = AccountWindow {
    ///     window: "five_hour".to_string(),
    ///     utilization: 0.9,
    ///     resets_at: Some(now - TimeDelta::minutes(1)),
    ///     status: WindowStatus::Allowed,
    ///     model_scope: None,
    ///     observed_at: now,
    /// };
    /// assert_eq!(window.effective_utilization(now), 0.0);
    /// ```
    pub fn effective_utilization(&self, now: DateTime<Utc>) -> f64 {
        match self.resets_at {
            Some(reset) if reset <= now => 0.0,
            _ => self.utilization,
        }
    }
}

/// The credential of a Provider Account, as injected into a provider process.
///
/// Its [`Debug`] output never shows the value, and it is not serializable.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::AccountCredential;
///
/// let credential = AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", "secret".to_string());
/// assert_eq!(credential.env_var(), "CLAUDE_CODE_OAUTH_TOKEN");
/// assert!(!format!("{credential:?}").contains("secret"));
/// ```
#[derive(Clone)]
pub struct AccountCredential {
    env_var: &'static str,
    value: String,
}

impl AccountCredential {
    /// Wrap a credential value with the environment variable carrying it.
    ///
    /// # Examples
    ///
    /// See [`AccountCredential`].
    pub fn new(env_var: &'static str, value: String) -> Self {
        Self { env_var, value }
    }

    /// Environment variable the credential is exposed as.
    ///
    /// # Examples
    ///
    /// See [`AccountCredential`].
    pub fn env_var(&self) -> &'static str {
        self.env_var
    }

    /// The raw credential value. Never log it.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::account::AccountCredential;
    ///
    /// let credential = AccountCredential::new("TOKEN", "value".to_string());
    /// assert_eq!(credential.expose(), "value");
    /// ```
    pub fn expose(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for AccountCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountCredential")
            .field("env_var", &self.env_var)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Collects the usage windows a provider observes during an invocation.
///
/// Only the last observation per `(window, model_scope)` is kept.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_core::account::{AccountWindow, RateLimitRecorder, WindowStatus};
///
/// let recorder = RateLimitRecorder::default();
/// let window = AccountWindow {
///     window: "five_hour".to_string(),
///     utilization: 0.2,
///     resets_at: None,
///     status: WindowStatus::Allowed,
///     model_scope: None,
///     observed_at: Utc::now(),
/// };
/// recorder.record(window.clone());
/// recorder.record(AccountWindow { utilization: 0.3, ..window });
/// let windows = recorder.take();
/// assert_eq!(windows.len(), 1);
/// assert_eq!(windows[0].utilization, 0.3);
/// assert!(recorder.take().is_empty());
/// ```
#[derive(Debug, Clone, Default)]
pub struct RateLimitRecorder(Arc<Mutex<Vec<AccountWindow>>>);

impl RateLimitRecorder {
    /// Record an observed window, replacing a previous one with the same
    /// `(window, model_scope)`.
    ///
    /// # Examples
    ///
    /// See [`RateLimitRecorder`].
    pub fn record(&self, window: AccountWindow) {
        let mut windows = self.0.lock().unwrap_or_else(|e| e.into_inner());
        windows.retain(|w| !(w.window == window.window && w.model_scope == window.model_scope));
        windows.push(window);
    }

    /// Drain every recorded window.
    ///
    /// # Examples
    ///
    /// See [`RateLimitRecorder`].
    pub fn take(&self) -> Vec<AccountWindow> {
        let mut windows = self.0.lock().unwrap_or_else(|e| e.into_inner());
        mem::take(&mut *windows)
    }
}

/// The Provider Account a single invocation runs under.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::{AccountCredential, AccountSession, RateLimitRecorder};
///
/// let session = AccountSession::new(
///     AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", "secret".to_string()),
///     RateLimitRecorder::default(),
/// );
/// assert_eq!(session.credential().env_var(), "CLAUDE_CODE_OAUTH_TOKEN");
/// assert!(!format!("{session:?}").contains("secret"));
/// ```
#[derive(Clone)]
pub struct AccountSession {
    credential: AccountCredential,
    recorder: RateLimitRecorder,
}

impl AccountSession {
    /// Build a session from a credential and the recorder observations go to.
    ///
    /// # Examples
    ///
    /// See [`AccountSession`].
    pub fn new(credential: AccountCredential, recorder: RateLimitRecorder) -> Self {
        Self {
            credential,
            recorder,
        }
    }

    /// The credential to inject.
    ///
    /// # Examples
    ///
    /// See [`AccountSession`].
    pub fn credential(&self) -> &AccountCredential {
        &self.credential
    }

    /// The recorder observed windows are pushed to.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::account::{AccountCredential, AccountSession, RateLimitRecorder};
    ///
    /// let recorder = RateLimitRecorder::default();
    /// let session = AccountSession::new(AccountCredential::new("T", "v".to_string()), recorder);
    /// assert!(session.recorder().take().is_empty());
    /// ```
    pub fn recorder(&self) -> &RateLimitRecorder {
        &self.recorder
    }
}

impl fmt::Debug for AccountSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountSession")
            .field("credential", &self.credential)
            .finish_non_exhaustive()
    }
}

/// Outcome of a successful live credential check.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::CredentialCheck;
///
/// let check = CredentialCheck::Valid { windows: Vec::new() };
/// assert!(check.windows().is_empty());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum CredentialCheck {
    /// The provider accepted the credential.
    Valid {
        /// Windows reported with the answer.
        windows: Vec<AccountWindow>,
    },
    /// The credential is valid but currently rate limited.
    Limited {
        /// Windows reported with the answer, at least one rejected.
        windows: Vec<AccountWindow>,
    },
}

impl CredentialCheck {
    /// Windows reported by the check.
    ///
    /// # Examples
    ///
    /// See [`CredentialCheck`].
    pub fn windows(&self) -> &[AccountWindow] {
        match self {
            Self::Valid { windows } | Self::Limited { windows } => windows,
        }
    }
}

/// Errors raised while validating or checking an account credential.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::AccountError;
///
/// let err = AccountError::Unauthorized { status: 401 };
/// assert!(err.to_string().contains("401"));
/// ```
#[derive(Debug, Error)]
pub enum AccountError {
    /// The credential format was rejected before any call.
    #[error("invalid credential: {0}")]
    InvalidCredential(String),
    /// The provider rejected the credential.
    #[error("credential rejected by provider (HTTP {status})")]
    Unauthorized {
        /// HTTP status returned (401 or 403).
        status: u16,
    },
    /// The check could not complete (network error, unexpected status).
    #[error("credential check failed: {0}")]
    CheckFailed(String),
}

/// A form field shown when adding an account of a kind.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::{AccountKind, ClaudeSubscriptionKind};
///
/// let fields = ClaudeSubscriptionKind::new().form_fields();
/// assert_eq!(fields[0].name, "token");
/// assert!(fields[0].secret);
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct AccountFormField {
    /// Field identifier.
    pub name: &'static str,
    /// Human-readable label.
    pub label: &'static str,
    /// Whether the value is a secret (write-only, masked).
    pub secret: bool,
    /// Help text shown next to the field.
    pub help: &'static str,
}

/// Future returned by [`AccountKind::check_credential`].
pub type AccountCheckFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CredentialCheck, AccountError>> + Send + 'a>>;

/// A kind of Provider Account.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::{AccountKind, ClaudeSubscriptionKind};
///
/// let kind = ClaudeSubscriptionKind::new();
/// assert_eq!(kind.id(), "claude_subscription");
/// let credential = kind.credential("sk-ant-oat01-xxxx");
/// assert_eq!(credential.env_var(), "CLAUDE_CODE_OAUTH_TOKEN");
/// ```
pub trait AccountKind: Send + Sync {
    /// Stable identifier, stored in the account row.
    fn id(&self) -> &'static str;

    /// Human-readable name.
    fn display_name(&self) -> &'static str;

    /// Fields of the add form.
    fn form_fields(&self) -> Vec<AccountFormField>;

    /// Check the credential format without any network call.
    ///
    /// # Errors
    ///
    /// Returns [`AccountError::InvalidCredential`] when the format is wrong.
    fn validate_credential(&self, raw: &str) -> Result<(), AccountError>;

    /// Build the credential injected into provider processes.
    fn credential(&self, secret: &str) -> AccountCredential;

    /// Check the credential against the provider.
    ///
    /// # Errors
    ///
    /// Returns [`AccountError::Unauthorized`] when the provider rejects it and
    /// [`AccountError::CheckFailed`] when the check cannot complete.
    fn check_credential<'a>(&'a self, secret: &'a str) -> AccountCheckFuture<'a>;
}

const CLAUDE_TOKEN_PREFIX: &str = "sk-ant-oat01-";
const CLAUDE_TOKEN_MIN_LEN: usize = 40;
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const CHECK_MODEL: &str = "claude-haiku-4-5-20251001";
const MAX_ERROR_BODY: usize = 200;

/// Claude Pro/Max subscription, authenticated by a `claude setup-token` token.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::{AccountKind, ClaudeSubscriptionKind};
///
/// let kind = ClaudeSubscriptionKind::new();
/// assert_eq!(kind.id(), ClaudeSubscriptionKind::ID);
/// ```
#[derive(Debug, Clone)]
pub struct ClaudeSubscriptionKind {
    api_base: String,
    client: Client,
}

impl ClaudeSubscriptionKind {
    /// Kind identifier.
    pub const ID: &'static str = "claude_subscription";

    /// Environment variable the Claude CLI reads the token from.
    pub const TOKEN_ENV: &'static str = "CLAUDE_CODE_OAUTH_TOKEN";

    /// Kind talking to `https://api.anthropic.com`.
    ///
    /// # Examples
    ///
    /// See [`ClaudeSubscriptionKind`].
    pub fn new() -> Self {
        Self::with_api_base("https://api.anthropic.com")
    }

    /// Kind talking to another API base URL (tests, proxies).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::account::ClaudeSubscriptionKind;
    ///
    /// let kind = ClaudeSubscriptionKind::with_api_base("http://127.0.0.1:8080");
    /// # let _ = kind;
    /// ```
    pub fn with_api_base(api_base: &str) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client: Client::new(),
        }
    }
}

impl Default for ClaudeSubscriptionKind {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountKind for ClaudeSubscriptionKind {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn display_name(&self) -> &'static str {
        "Claude subscription (Pro/Max)"
    }

    fn form_fields(&self) -> Vec<AccountFormField> {
        vec![AccountFormField {
            name: "token",
            label: "OAuth token",
            secret: true,
            help: "Run `claude setup-token` and paste the sk-ant-oat01-... token",
        }]
    }

    fn validate_credential(&self, raw: &str) -> Result<(), AccountError> {
        let token = raw.trim();
        let well_formed = token.starts_with(CLAUDE_TOKEN_PREFIX)
            && token.len() >= CLAUDE_TOKEN_MIN_LEN
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if well_formed {
            Ok(())
        } else {
            Err(AccountError::InvalidCredential(
                "expected a `claude setup-token` token (sk-ant-oat01-...)".to_string(),
            ))
        }
    }

    fn credential(&self, secret: &str) -> AccountCredential {
        AccountCredential::new(Self::TOKEN_ENV, secret.trim().to_string())
    }

    fn check_credential<'a>(&'a self, secret: &'a str) -> AccountCheckFuture<'a> {
        Box::pin(async move {
            let body = json!({
                "model": CHECK_MODEL,
                "max_tokens": 1,
                "system": "You are Claude Code, Anthropic's official CLI for Claude.",
                "messages": [{"role": "user", "content": "ping"}],
            });
            let response = self
                .client
                .post(format!("{}/v1/messages", self.api_base))
                .timeout(CHECK_TIMEOUT)
                .bearer_auth(secret.trim())
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", "oauth-2025-04-20")
                .json(&body)
                .send()
                .await
                .map_err(|e| AccountError::CheckFailed(e.without_url().to_string()))?;

            let status = response.status();
            let now = Utc::now();
            if status.is_success() {
                return Ok(CredentialCheck::Valid {
                    windows: windows_from_headers(response.headers(), now, false),
                });
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Ok(CredentialCheck::Limited {
                    windows: windows_from_headers(response.headers(), now, true),
                });
            }
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                return Err(AccountError::Unauthorized {
                    status: status.as_u16(),
                });
            }
            let text = match response.text().await {
                Ok(text) => text,
                Err(e) => format!("<unreadable body: {e}>"),
            };
            let truncated: String = text.chars().take(MAX_ERROR_BODY).collect();
            Err(AccountError::CheckFailed(format!(
                "HTTP {}: {truncated}",
                status.as_u16()
            )))
        })
    }
}

fn header_str<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

fn unix_seconds(value: &str) -> Option<DateTime<Utc>> {
    let secs = value.parse::<i64>().ok()?;
    Utc.timestamp_opt(secs, 0).single()
}

/// Read the unified rate-limit headers into windows.
///
/// With `limited` set and no window header, builds one rejected `five_hour`
/// window from the generic reset headers.
fn windows_from_headers(
    headers: &HeaderMap,
    now: DateTime<Utc>,
    limited: bool,
) -> Vec<AccountWindow> {
    let mut windows = Vec::new();
    for (suffix, name) in [("5h", "five_hour"), ("7d", "seven_day")] {
        let prefix = format!("anthropic-ratelimit-unified-{suffix}");
        let utilization = header_str(headers, &format!("{prefix}-utilization"))
            .and_then(|v| v.parse::<f64>().ok());
        let status = header_str(headers, &format!("{prefix}-status"))
            .and_then(|v| v.parse::<WindowStatus>().ok());
        if utilization.is_none() && status.is_none() {
            continue;
        }
        let status = status.unwrap_or(WindowStatus::Allowed);
        let utilization = utilization
            .map(|u| if u > 1.0 { u / 100.0 } else { u })
            .unwrap_or(if status == WindowStatus::Rejected {
                1.0
            } else {
                0.0
            })
            .clamp(0.0, 1.0);
        windows.push(AccountWindow {
            window: name.to_string(),
            utilization,
            resets_at: header_str(headers, &format!("{prefix}-reset")).and_then(unix_seconds),
            status,
            model_scope: None,
            observed_at: now,
        });
    }

    if limited && windows.is_empty() {
        let resets_at = header_str(headers, "anthropic-ratelimit-unified-reset")
            .and_then(unix_seconds)
            .or_else(|| {
                header_str(headers, RETRY_AFTER.as_str())
                    .and_then(|v| v.parse::<i64>().ok())
                    .map(|secs| now + TimeDelta::seconds(secs))
            });
        windows.push(AccountWindow {
            window: "five_hour".to_string(),
            utilization: 1.0,
            resets_at,
            status: WindowStatus::Rejected,
            model_scope: None,
            observed_at: now,
        });
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_strategy::{AccountCandidate, LeastUtilized, select_account};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const GOOD_TOKEN: &str = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz0123456789_-AB";

    #[test]
    fn validate_credential_accepts_well_formed_token() {
        let kind = ClaudeSubscriptionKind::new();
        assert!(kind.validate_credential(GOOD_TOKEN).is_ok());
        assert!(
            kind.validate_credential(&format!("  {GOOD_TOKEN}\n"))
                .is_ok()
        );
    }

    #[test]
    fn validate_credential_rejects_wrong_prefix() {
        let kind = ClaudeSubscriptionKind::new();
        let token = GOOD_TOKEN.replace("oat01", "api03");
        assert!(matches!(
            kind.validate_credential(&token),
            Err(AccountError::InvalidCredential(_))
        ));
    }

    #[test]
    fn validate_credential_rejects_short_token() {
        let kind = ClaudeSubscriptionKind::new();
        assert!(kind.validate_credential("sk-ant-oat01-invalid").is_err());
        assert!(kind.validate_credential("").is_err());
    }

    #[test]
    fn validate_credential_rejects_bad_chars() {
        let kind = ClaudeSubscriptionKind::new();
        let token = format!("{GOOD_TOKEN}$é");
        assert!(kind.validate_credential(&token).is_err());
        let token = format!("{GOOD_TOKEN} abc");
        assert!(kind.validate_credential(&token).is_err());
    }

    #[test]
    fn credential_and_session_debug_are_redacted() {
        let credential = AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", GOOD_TOKEN.to_string());
        let debug = format!("{credential:?}");
        assert!(!debug.contains(GOOD_TOKEN));
        assert!(debug.contains("[REDACTED]"));
        let session = AccountSession::new(credential, RateLimitRecorder::default());
        assert!(!format!("{session:?}").contains(GOOD_TOKEN));
    }

    #[test]
    fn credential_trims_the_secret() {
        let kind = ClaudeSubscriptionKind::new();
        let credential = kind.credential(&format!("{GOOD_TOKEN}\n"));
        assert_eq!(credential.expose(), GOOD_TOKEN);
    }

    #[test]
    fn form_fields_have_one_secret_token() {
        let fields = ClaudeSubscriptionKind::new().form_fields();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "token");
        assert!(fields[0].secret);
        assert!(fields[0].help.contains("claude setup-token"));
    }

    /// Serve one HTTP request on a real local socket with a canned response.
    async fn stub_server(response: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(end) = text.find("\r\n\r\n") {
                    let content_length = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + content_length {
                        break;
                    }
                }
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        format!("http://{addr}")
    }

    fn http_response(status: &str, headers: &[(&str, String)], body: &str) -> String {
        let mut out = format!("HTTP/1.1 {status}\r\n");
        for (name, value) in headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str(&format!(
            "content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        ));
        out
    }

    #[tokio::test]
    async fn check_credential_valid_reads_unified_windows() {
        let reset = Utc::now().timestamp() + 3600;
        let response = http_response(
            "200 OK",
            &[
                (
                    "anthropic-ratelimit-unified-5h-utilization",
                    "0.42".to_string(),
                ),
                ("anthropic-ratelimit-unified-5h-reset", reset.to_string()),
                (
                    "anthropic-ratelimit-unified-5h-status",
                    "allowed".to_string(),
                ),
                (
                    "anthropic-ratelimit-unified-7d-utilization",
                    "0.8".to_string(),
                ),
                (
                    "anthropic-ratelimit-unified-7d-status",
                    "allowed_warning".to_string(),
                ),
            ],
            "{}",
        );
        let base = stub_server(response).await;
        let kind = ClaudeSubscriptionKind::with_api_base(&base);
        let check = kind.check_credential(GOOD_TOKEN).await.unwrap();
        let CredentialCheck::Valid { windows } = check else {
            panic!("expected Valid, got {check:?}");
        };
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].window, "five_hour");
        assert!((windows[0].utilization - 0.42).abs() < 1e-9);
        assert_eq!(windows[0].resets_at.unwrap().timestamp(), reset);
        assert_eq!(windows[1].window, "seven_day");
        assert_eq!(windows[1].status, WindowStatus::AllowedWarning);
    }

    #[tokio::test]
    async fn check_credential_429_is_limited_with_rejected_window() {
        let reset = Utc::now().timestamp() + 1800;
        let response = http_response(
            "429 Too Many Requests",
            &[("anthropic-ratelimit-unified-reset", reset.to_string())],
            "{\"error\":\"rate_limited\"}",
        );
        let base = stub_server(response).await;
        let kind = ClaudeSubscriptionKind::with_api_base(&base);
        let check = kind.check_credential(GOOD_TOKEN).await.unwrap();
        let CredentialCheck::Limited { windows } = check else {
            panic!("expected Limited, got {check:?}");
        };
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window, "five_hour");
        assert_eq!(windows[0].status, WindowStatus::Rejected);
        assert_eq!(windows[0].utilization, 1.0);
        assert_eq!(windows[0].resets_at.unwrap().timestamp(), reset);
    }

    #[tokio::test]
    async fn check_credential_401_is_unauthorized() {
        let response = http_response("401 Unauthorized", &[], "{}");
        let base = stub_server(response).await;
        let kind = ClaudeSubscriptionKind::with_api_base(&base);
        let err = kind.check_credential(GOOD_TOKEN).await.unwrap_err();
        assert!(matches!(err, AccountError::Unauthorized { status: 401 }));
        assert!(!err.to_string().contains(GOOD_TOKEN));
    }

    #[tokio::test]
    async fn check_credential_500_is_check_failed() {
        let response = http_response("500 Internal Server Error", &[], "boom");
        let base = stub_server(response).await;
        let kind = ClaudeSubscriptionKind::with_api_base(&base);
        let err = kind.check_credential(GOOD_TOKEN).await.unwrap_err();
        let AccountError::CheckFailed(message) = err else {
            panic!("expected CheckFailed");
        };
        assert!(message.contains("HTTP 500"));
        assert!(message.contains("boom"));
    }

    #[tokio::test]
    async fn check_credential_unreachable_is_check_failed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let kind = ClaudeSubscriptionKind::with_api_base(&format!("http://{addr}"));
        let err = kind.check_credential(GOOD_TOKEN).await.unwrap_err();
        assert!(matches!(err, AccountError::CheckFailed(_)));
    }

    /// A kind limited by a dollar budget, to keep [`AccountKind`] honest
    /// beyond Claude subscriptions.
    struct BudgetKind;

    impl AccountKind for BudgetKind {
        fn id(&self) -> &'static str {
            "budget"
        }

        fn display_name(&self) -> &'static str {
            "Budget"
        }

        fn form_fields(&self) -> Vec<AccountFormField> {
            vec![AccountFormField {
                name: "api_key",
                label: "API key",
                secret: true,
                help: "Paste the API key",
            }]
        }

        fn validate_credential(&self, raw: &str) -> Result<(), AccountError> {
            if raw.is_empty() {
                Err(AccountError::InvalidCredential("empty".to_string()))
            } else {
                Ok(())
            }
        }

        fn credential(&self, secret: &str) -> AccountCredential {
            AccountCredential::new("BUDGET_API_KEY", secret.to_string())
        }

        fn check_credential<'a>(&'a self, _secret: &'a str) -> AccountCheckFuture<'a> {
            Box::pin(async {
                Ok(CredentialCheck::Valid {
                    windows: vec![budget_window(0.5, WindowStatus::Allowed)],
                })
            })
        }
    }

    fn budget_window(utilization: f64, status: WindowStatus) -> AccountWindow {
        AccountWindow {
            window: "monthly_budget_usd".to_string(),
            utilization,
            resets_at: Some(Utc::now() + TimeDelta::days(10)),
            status,
            model_scope: None,
            observed_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn budget_kind_goes_through_select_account() {
        let kind = BudgetKind;
        assert!(kind.validate_credential("").is_err());
        let check = kind.check_credential("key").await.unwrap();
        let spent = AccountCandidate {
            id: "a".to_string(),
            name: "spent".to_string(),
            priority: 1,
            max_concurrency: None,
            running_steps: 0,
            windows: vec![budget_window(1.0, WindowStatus::Rejected)],
        };
        let fresh = AccountCandidate {
            id: "b".to_string(),
            name: "fresh".to_string(),
            priority: 2,
            max_concurrency: None,
            running_steps: 0,
            windows: check.windows().to_vec(),
        };
        let candidates = [spent, fresh];
        let selected =
            select_account(&LeastUtilized, &candidates, "any-model", Utc::now()).unwrap();
        assert_eq!(selected.name, "fresh");
        assert_eq!(kind.credential("key").env_var(), "BUDGET_API_KEY");
    }
}
