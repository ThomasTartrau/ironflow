//! Parsing of the `rate_limit_event` lines of the Claude CLI `stream-json` output.
//!
//! When a Claude subscription is close to (or over) one of its usage windows,
//! the CLI emits a line such as:
//!
//! ```json
//! {"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"five_hour","utilization":0.91,"resetsAt":1767225600}}
//! ```
//!
//! [`record_rate_limits`] turns those lines into [`AccountWindow`]s and hands
//! them to the recorder of the invocation's [`AccountSession`](crate::account::AccountSession).
//!
//! # Examples
//!
//! ```
//! use chrono::Utc;
//! use ironflow_core::providers::claude::rate_limit_event::collect_rate_limit_events;
//!
//! let stdout = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour","utilization":0.3}}"#;
//! let windows = collect_rate_limit_events(stdout, Utc::now());
//! assert_eq!(windows[0].window, "five_hour");
//! ```

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, from_str};
use tracing::{debug, warn};

use crate::account::{AccountWindow, WindowStatus};
use crate::provider::AgentConfig;

/// Timestamps above this value are milliseconds, not seconds.
const MILLIS_THRESHOLD: f64 = 1e12;

/// Parse one `stream-json` line into a window, `None` when it is not a
/// usable `rate_limit_event`.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use serde_json::json;
/// use ironflow_core::providers::claude::rate_limit_event::parse_rate_limit_event;
///
/// let line = json!({
///     "type": "rate_limit_event",
///     "rate_limit_info": {"status": "rejected", "rateLimitType": "seven_day_opus"}
/// });
/// let window = parse_rate_limit_event(&line, Utc::now()).expect("window");
/// assert_eq!(window.window, "seven_day");
/// assert_eq!(window.model_scope.as_deref(), Some("opus"));
/// assert_eq!(window.utilization, 1.0);
/// ```
pub fn parse_rate_limit_event(line: &Value, now: DateTime<Utc>) -> Option<AccountWindow> {
    if line.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
        return None;
    }
    let info = line
        .get("rate_limit_info")
        .filter(|v| v.is_object())
        .unwrap_or(line);

    let status_raw = info.get("status").and_then(Value::as_str);
    let status = match status_raw {
        Some("allowed") => WindowStatus::Allowed,
        Some("allowed_warning") => WindowStatus::AllowedWarning,
        Some("rejected") => WindowStatus::Rejected,
        other => {
            warn!(status = ?other, "rate_limit_event with unknown status, skipped");
            return None;
        }
    };

    let limit_type = info
        .get("rateLimitType")
        .or_else(|| info.get("rate_limit_type"))
        .and_then(Value::as_str);
    let (window, model_scope) = match limit_type {
        Some("five_hour") => ("five_hour".to_string(), None),
        Some("seven_day") => ("seven_day".to_string(), None),
        Some("seven_day_opus") => ("seven_day".to_string(), Some("opus".to_string())),
        Some("seven_day_sonnet") => ("seven_day".to_string(), Some("sonnet".to_string())),
        Some(other) => (other.to_string(), None),
        None => {
            debug!("rate_limit_event without rateLimitType, skipped");
            return None;
        }
    };

    let utilization = match info.get("utilization").and_then(Value::as_f64) {
        Some(raw) => {
            let fraction = if raw > 1.0 { raw / 100.0 } else { raw };
            fraction.clamp(0.0, 1.0)
        }
        None if status == WindowStatus::Rejected => 1.0,
        None => {
            debug!(window = %window, "rate_limit_event without utilization, skipped");
            return None;
        }
    };

    let resets_at = info
        .get("resetsAt")
        .or_else(|| info.get("resets_at"))
        .and_then(parse_reset);

    Some(AccountWindow {
        window,
        utilization,
        resets_at,
        status,
        model_scope,
        observed_at: now,
    })
}

fn parse_reset(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(text) = value.as_str() {
        return DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|dt| dt.with_timezone(&Utc));
    }
    let number = value.as_f64()?;
    if number > MILLIS_THRESHOLD {
        Utc.timestamp_millis_opt(number as i64).single()
    } else {
        Utc.timestamp_opt(number as i64, 0).single()
    }
}

/// Parse every line of a `stream-json` output, keeping the last event per
/// `(window, model_scope)`.
///
/// # Examples
///
/// See the [module documentation](self).
pub fn collect_rate_limit_events(stdout: &str, now: DateTime<Utc>) -> Vec<AccountWindow> {
    let mut windows: Vec<AccountWindow> = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("rate_limit_event") {
            continue;
        }
        let Ok(parsed) = from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(window) = parse_rate_limit_event(&parsed, now) {
            windows.retain(|w| !(w.window == window.window && w.model_scope == window.model_scope));
            windows.push(window);
        }
    }
    windows
}

/// Push the windows found in `stdout` to the recorder of `config.account`.
///
/// Does nothing when the invocation runs without a Provider Account.
///
/// # Examples
///
/// ```
/// use ironflow_core::account::{AccountCredential, AccountSession, RateLimitRecorder};
/// use ironflow_core::provider::AgentConfig;
/// use ironflow_core::providers::claude::rate_limit_event::record_rate_limits;
///
/// let recorder = RateLimitRecorder::default();
/// let config = AgentConfig::new("x").account_session(AccountSession::new(
///     AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", "t".to_string()),
///     recorder.clone(),
/// ));
/// let stdout = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour","utilization":0.5}}"#;
/// record_rate_limits(&config, stdout);
/// assert_eq!(recorder.take().len(), 1);
/// ```
pub fn record_rate_limits(config: &AgentConfig, stdout: &str) {
    let Some(session) = &config.account else {
        return;
    };
    for window in collect_rate_limit_events(stdout, Utc::now()) {
        session.recorder().record(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{AccountCredential, AccountSession, RateLimitRecorder};
    use serde_json::json;

    fn event(info: Value) -> Value {
        json!({"type": "rate_limit_event", "rate_limit_info": info})
    }

    #[test]
    fn rate_limit_event_parses_five_hour_window() {
        let now = Utc::now();
        let line = event(json!({
            "status": "allowed_warning",
            "rateLimitType": "five_hour",
            "utilization": 0.91,
            "resetsAt": 1_767_225_600
        }));
        let window = parse_rate_limit_event(&line, now).unwrap();
        assert_eq!(window.window, "five_hour");
        assert_eq!(window.model_scope, None);
        assert_eq!(window.status, WindowStatus::AllowedWarning);
        assert!((window.utilization - 0.91).abs() < 1e-9);
        assert_eq!(window.resets_at.unwrap().timestamp(), 1_767_225_600);
        assert_eq!(window.observed_at, now);
    }

    #[test]
    fn rate_limit_event_maps_seven_day_opus_to_model_scope() {
        let line = event(
            json!({"status": "allowed", "rateLimitType": "seven_day_opus", "utilization": 0.4}),
        );
        let window = parse_rate_limit_event(&line, Utc::now()).unwrap();
        assert_eq!(window.window, "seven_day");
        assert_eq!(window.model_scope.as_deref(), Some("opus"));

        let line = event(
            json!({"status": "allowed", "rate_limit_type": "seven_day_sonnet", "utilization": 0.4}),
        );
        let window = parse_rate_limit_event(&line, Utc::now()).unwrap();
        assert_eq!(window.model_scope.as_deref(), Some("sonnet"));

        let line =
            event(json!({"status": "allowed", "rateLimitType": "overage", "utilization": 0.1}));
        let window = parse_rate_limit_event(&line, Utc::now()).unwrap();
        assert_eq!(window.window, "overage");
        assert_eq!(window.model_scope, None);
    }

    #[test]
    fn rate_limit_event_rejected_status_without_utilization() {
        let line = event(json!({"status": "rejected", "rateLimitType": "five_hour"}));
        let window = parse_rate_limit_event(&line, Utc::now()).unwrap();
        assert_eq!(window.status, WindowStatus::Rejected);
        assert_eq!(window.utilization, 1.0);
        assert_eq!(window.resets_at, None);

        let line = event(json!({"status": "allowed", "rateLimitType": "five_hour"}));
        assert!(parse_rate_limit_event(&line, Utc::now()).is_none());
    }

    #[test]
    fn rate_limit_event_percent_utilization_is_normalized() {
        let line =
            event(json!({"status": "allowed", "rateLimitType": "five_hour", "utilization": 42}));
        let window = parse_rate_limit_event(&line, Utc::now()).unwrap();
        assert!((window.utilization - 0.42).abs() < 1e-9);

        let line =
            event(json!({"status": "rejected", "rateLimitType": "five_hour", "utilization": 250}));
        assert_eq!(
            parse_rate_limit_event(&line, Utc::now())
                .unwrap()
                .utilization,
            1.0
        );

        let line =
            event(json!({"status": "allowed", "rateLimitType": "five_hour", "utilization": -0.5}));
        assert_eq!(
            parse_rate_limit_event(&line, Utc::now())
                .unwrap()
                .utilization,
            0.0
        );
    }

    #[test]
    fn rate_limit_event_iso_resets_at() {
        let line = event(json!({
            "status": "allowed",
            "rateLimitType": "seven_day",
            "utilization": 0.1,
            "resetsAt": "2026-10-01T12:00:00Z"
        }));
        let window = parse_rate_limit_event(&line, Utc::now()).unwrap();
        assert_eq!(
            window.resets_at.unwrap().to_rfc3339(),
            "2026-10-01T12:00:00+00:00"
        );

        let line = event(json!({
            "status": "allowed",
            "rateLimitType": "seven_day",
            "utilization": 0.1,
            "resetsAt": 1_767_225_600_000_i64
        }));
        let window = parse_rate_limit_event(&line, Utc::now()).unwrap();
        assert_eq!(window.resets_at.unwrap().timestamp(), 1_767_225_600);
    }

    #[test]
    fn rate_limit_event_unknown_status_is_skipped() {
        let line =
            event(json!({"status": "throttled", "rateLimitType": "five_hour", "utilization": 0.5}));
        assert!(parse_rate_limit_event(&line, Utc::now()).is_none());
        let line = event(json!({"rateLimitType": "five_hour", "utilization": 0.5}));
        assert!(parse_rate_limit_event(&line, Utc::now()).is_none());
    }

    #[test]
    fn rate_limit_event_non_matching_type_returns_none() {
        let line = json!({"type": "assistant", "status": "allowed", "rateLimitType": "five_hour", "utilization": 0.5});
        assert!(parse_rate_limit_event(&line, Utc::now()).is_none());
        assert!(parse_rate_limit_event(&json!({}), Utc::now()).is_none());
    }

    #[test]
    fn rate_limit_event_top_level_fields_are_accepted() {
        let line = json!({"type": "rate_limit_event", "status": "allowed", "rateLimitType": "five_hour", "utilization": 0.25});
        let window = parse_rate_limit_event(&line, Utc::now()).unwrap();
        assert!((window.utilization - 0.25).abs() < 1e-9);
    }

    #[test]
    fn rate_limit_event_collect_keeps_last_per_window() {
        let stdout = [
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour","utilization":0.2}}"#,
            "not json rate_limit_event",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"seven_day_opus","utilization":0.6}}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"five_hour","utilization":0.85}}"#,
            r#"{"type":"result","result":"done"}"#,
        ]
        .join("\n");
        let windows = collect_rate_limit_events(&stdout, Utc::now());
        assert_eq!(windows.len(), 2);
        let five = windows.iter().find(|w| w.window == "five_hour").unwrap();
        assert_eq!(five.status, WindowStatus::AllowedWarning);
        assert!((five.utilization - 0.85).abs() < 1e-9);
        assert!(
            windows
                .iter()
                .any(|w| w.model_scope.as_deref() == Some("opus"))
        );
        assert!(collect_rate_limit_events("", Utc::now()).is_empty());
    }

    #[test]
    fn rate_limit_event_recorded_only_with_account_session() {
        let stdout = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour","utilization":0.5}}"#;

        let without = AgentConfig::new("x");
        record_rate_limits(&without, stdout);

        let recorder = RateLimitRecorder::default();
        let with = AgentConfig::new("x").account_session(AccountSession::new(
            AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", "t".to_string()),
            recorder.clone(),
        ));
        record_rate_limits(&with, stdout);
        let windows = recorder.take();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window, "five_hour");
    }
}
