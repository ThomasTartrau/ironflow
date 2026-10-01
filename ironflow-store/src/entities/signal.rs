//! Signal entities -- external messages that resume waiting runs.
//!
//! A signal is named (what happened, e.g. `"ci.pipeline_finished"`) and keyed
//! (which occurrence, e.g. a commit SHA). A run waiting through
//! `ctx.wait_for_signal` on the same `(name, key)` pair is resumed when a
//! signal with a valid payload is delivered.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// A persisted signal.
///
/// Signals are kept for a retention period after `received_at`, so a run that
/// opens its wait step after the signal arrived still finds it.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::Signal;
/// use serde_json::json;
/// use uuid::Uuid;
///
/// let signal = Signal {
///     id: Uuid::now_v7(),
///     name: "ci.pipeline_finished".to_string(),
///     key: "4f2a9c1".to_string(),
///     payload: json!({"status": "success"}),
///     idempotency_id: Some("delivery-42".to_string()),
///     received_at: Utc::now(),
/// };
/// assert_eq!(signal.key, "4f2a9c1");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    /// Unique signal ID (UUID v7).
    pub id: Uuid,
    /// Signal name, e.g. `"ci.pipeline_finished"`.
    pub name: String,
    /// Occurrence key, e.g. a commit SHA.
    pub key: String,
    /// JSON payload carried by the signal.
    pub payload: Value,
    /// Caller-provided deduplication ID. A second signal with the same ID is
    /// not stored nor delivered again.
    pub idempotency_id: Option<String>,
    /// When the signal was received.
    pub received_at: DateTime<Utc>,
}

/// Request to store a new signal.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::NewSignal;
/// use serde_json::json;
///
/// let signal = NewSignal {
///     name: "ci.pipeline_finished".to_string(),
///     key: "4f2a9c1".to_string(),
///     payload: json!({"status": "success"}),
///     idempotency_id: None,
/// };
/// assert!(signal.idempotency_id.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSignal {
    /// Signal name.
    pub name: String,
    /// Occurrence key.
    pub key: String,
    /// JSON payload.
    pub payload: Value,
    /// Optional deduplication ID.
    pub idempotency_id: Option<String>,
}

/// Outcome of [`SignalStore::insert_signal`](crate::signal_store::SignalStore::insert_signal).
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::{Signal, SignalInsert};
/// use serde_json::json;
/// use uuid::Uuid;
///
/// let signal = Signal {
///     id: Uuid::now_v7(),
///     name: "demo.done".to_string(),
///     key: "k1".to_string(),
///     payload: json!({}),
///     idempotency_id: Some("d-1".to_string()),
///     received_at: Utc::now(),
/// };
/// let insert = SignalInsert::Duplicate(signal);
/// assert!(insert.is_duplicate());
/// assert_eq!(insert.signal().key, "k1");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SignalInsert {
    /// The signal was stored.
    Created(Signal),
    /// A signal with the same idempotency ID already existed; it is returned
    /// unchanged and nothing was stored.
    Duplicate(Signal),
}

impl SignalInsert {
    /// The stored signal, new or pre-existing.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_store::entities::{Signal, SignalInsert};
    /// use serde_json::json;
    /// use uuid::Uuid;
    ///
    /// let signal = Signal {
    ///     id: Uuid::now_v7(),
    ///     name: "demo.done".to_string(),
    ///     key: "k1".to_string(),
    ///     payload: json!({}),
    ///     idempotency_id: None,
    ///     received_at: Utc::now(),
    /// };
    /// let insert = SignalInsert::Created(signal.clone());
    /// assert_eq!(insert.signal(), &signal);
    /// ```
    pub fn signal(&self) -> &Signal {
        match self {
            SignalInsert::Created(signal) | SignalInsert::Duplicate(signal) => signal,
        }
    }

    /// Whether the insert hit an existing idempotency ID.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_store::entities::{Signal, SignalInsert};
    /// use serde_json::json;
    /// use uuid::Uuid;
    ///
    /// let signal = Signal {
    ///     id: Uuid::now_v7(),
    ///     name: "demo.done".to_string(),
    ///     key: "k1".to_string(),
    ///     payload: json!({}),
    ///     idempotency_id: None,
    ///     received_at: Utc::now(),
    /// };
    /// assert!(!SignalInsert::Created(signal).is_duplicate());
    /// ```
    pub fn is_duplicate(&self) -> bool {
        matches!(self, SignalInsert::Duplicate(_))
    }
}

/// Filter criteria for listing signals.
///
/// Every field is optional; set fields are combined with AND and match exactly.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::SignalFilter;
///
/// let filter = SignalFilter {
///     name: Some("ci.pipeline_finished".to_string()),
///     ..SignalFilter::default()
/// };
/// assert!(filter.key.is_none());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalFilter {
    /// Exact signal name.
    pub name: Option<String>,
    /// Exact occurrence key.
    pub key: Option<String>,
}

/// Outcome of [`SignalStore::resolve_signal_step`](crate::signal_store::SignalStore::resolve_signal_step).
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::SignalStepResolution;
/// use uuid::Uuid;
///
/// # fn main() -> Result<(), serde_json::Error> {
/// let resolution = SignalStepResolution::Resolved {
///     run_id: Uuid::now_v7(),
///     run_resumed: true,
/// };
/// let json = serde_json::to_value(&resolution)?;
/// assert_eq!(json["outcome"], "resolved");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SignalStepResolution {
    /// The step was waiting and is now completed with the given output.
    Resolved {
        /// Run owning the step.
        run_id: Uuid,
        /// `true` when the run was `Sleeping` and went back to `Pending`.
        /// `false` when the run was still executing (it will see the step
        /// completed on its own).
        run_resumed: bool,
    },
    /// The step was no longer waiting: another delivery or a timeout won.
    NotWaiting {
        /// Output recorded by whoever resolved the step first.
        output: Option<Value>,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;

    fn signal() -> Signal {
        Signal {
            id: Uuid::now_v7(),
            name: "demo.done".to_string(),
            key: "k1".to_string(),
            payload: json!({"ok": true}),
            idempotency_id: None,
            received_at: Utc::now(),
        }
    }

    #[test]
    fn signal_insert_exposes_the_signal() {
        let s = signal();
        assert_eq!(SignalInsert::Created(s.clone()).signal(), &s);
        assert_eq!(SignalInsert::Duplicate(s.clone()).signal(), &s);
        assert!(!SignalInsert::Created(s.clone()).is_duplicate());
        assert!(SignalInsert::Duplicate(s).is_duplicate());
    }

    #[test]
    fn signal_step_resolution_serde_roundtrip() {
        let resolved = SignalStepResolution::Resolved {
            run_id: Uuid::now_v7(),
            run_resumed: false,
        };
        let json = to_value(&resolved).unwrap();
        assert_eq!(json["outcome"], "resolved");
        assert_eq!(from_value::<SignalStepResolution>(json).unwrap(), resolved);

        let not_waiting = SignalStepResolution::NotWaiting {
            output: Some(json!({"timed_out": true})),
        };
        let json = to_value(&not_waiting).unwrap();
        assert_eq!(json["outcome"], "not_waiting");
        assert_eq!(
            from_value::<SignalStepResolution>(json).unwrap(),
            not_waiting
        );
    }

    #[test]
    fn signal_serde_roundtrip() {
        let s = signal();
        let back: Signal = from_value(to_value(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
