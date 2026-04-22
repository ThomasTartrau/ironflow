//! Strongly-typed event kind enum for domain events.

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr};

/// Strongly-typed event kind matching domain event variants.
///
/// Serializes to/from `snake_case` strings (e.g. `"run_status_changed"`).
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::EventKind;
///
/// let kind: EventKind = "run_status_changed".parse().unwrap();
/// assert_eq!(kind, EventKind::RunStatusChanged);
/// assert_eq!(kind.as_str(), "run_status_changed");
/// ```
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum EventKind {
    /// A new run was created.
    RunCreated,
    /// A run changed status.
    RunStatusChanged,
    /// A run failed.
    RunFailed,
    /// A step completed successfully.
    StepCompleted,
    /// A step failed.
    StepFailed,
    /// A run is waiting for human approval.
    ApprovalRequested,
    /// A run was approved.
    ApprovalGranted,
    /// A run was rejected.
    ApprovalRejected,
    /// A log line was emitted by a running step.
    LogLine,
    /// A user signed in.
    UserSignedIn,
    /// A new user signed up.
    UserSignedUp,
    /// A user signed out.
    UserSignedOut,
}

impl EventKind {
    /// All known event kinds.
    pub const ALL: &'static [EventKind] = &[
        Self::RunCreated,
        Self::RunStatusChanged,
        Self::RunFailed,
        Self::StepCompleted,
        Self::StepFailed,
        Self::ApprovalRequested,
        Self::ApprovalGranted,
        Self::ApprovalRejected,
        Self::LogLine,
        Self::UserSignedIn,
        Self::UserSignedUp,
        Self::UserSignedOut,
    ];

    /// Returns the wire-format string for this kind.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::EventKind;
    ///
    /// assert_eq!(EventKind::RunCreated.as_str(), "run_created");
    /// assert_eq!(EventKind::StepFailed.as_str(), "step_failed");
    /// ```
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_roundtrip_via_str() {
        for kind in EventKind::ALL {
            let s = kind.as_str();
            let parsed: EventKind = s.parse().unwrap();
            assert_eq!(*kind, parsed);
        }
    }

    #[test]
    fn all_has_correct_count() {
        assert_eq!(EventKind::ALL.len(), 12);
    }

    #[test]
    fn display_matches_as_str() {
        for kind in EventKind::ALL {
            assert_eq!(kind.to_string(), kind.as_str());
        }
    }

    #[test]
    fn parse_unknown_returns_error() {
        assert!("bogus".parse::<EventKind>().is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let kind = EventKind::RunStatusChanged;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"run_status_changed\"");
        let back: EventKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn serde_all_variants() {
        for kind in EventKind::ALL {
            let json = serde_json::to_string(kind).unwrap();
            let back: EventKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, back);
        }
    }
}
