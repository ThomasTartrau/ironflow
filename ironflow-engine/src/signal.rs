//! Signals -- external messages, named and keyed, that resume waiting runs.
//!
//! A workflow waits with
//! [`WorkflowContext::wait_for_signal`](crate::context::WorkflowContext::wait_for_signal);
//! a producer (a webhook handler, a script, another service) delivers with
//! [`Engine::send_signal`](crate::engine::Engine::send_signal) or
//! `POST /api/v1/signals`. Another workflow delivers with
//! [`WorkflowContext::send_signal`](crate::context::WorkflowContext::send_signal),
//! for the signals its handler declares in
//! [`sent_signals`](crate::handler::WorkflowHandler::sent_signals).
//!
//! The **name** says what happened (`"ci.pipeline_finished"`), the **key**
//! says which occurrence (a commit SHA). Every run waiting on the same
//! `(name, key)` pair receives the signal.
//!
//! # Examples
//!
//! ```
//! use ironflow_engine::signal::Signal;
//! use schemars::JsonSchema;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize, JsonSchema)]
//! struct PipelineFinished {
//!     status: String,
//! }
//!
//! impl Signal for PipelineFinished {
//!     const NAME: &'static str = "ci.pipeline_finished";
//! }
//!
//! assert_eq!(PipelineFinished::NAME, "ci.pipeline_finished");
//! ```

use std::future::Future;
use std::pin::Pin;

use jsonschema::validator_for;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use ironflow_store::entities::{NewSignal, Signal as StoredSignal};

use crate::error::EngineError;

/// A typed signal a workflow can wait for.
///
/// The payload is the type itself: it is serialized when sent, validated
/// against its JSON schema on delivery, and deserialized for the waiting
/// handler.
///
/// # Examples
///
/// ```
/// use ironflow_engine::signal::Signal;
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize, JsonSchema)]
/// struct PipelineFinished {
///     status: String,
///     pipeline_id: u64,
/// }
///
/// impl Signal for PipelineFinished {
///     const NAME: &'static str = "ci.pipeline_finished";
/// }
/// ```
pub trait Signal: DeserializeOwned + Serialize + JsonSchema {
    /// Signal name, e.g. `"ci.pipeline_finished"`. Must not be empty.
    const NAME: &'static str;
}

/// Maximum length, in characters, of a signal name, key or idempotency ID.
pub const MAX_SIGNAL_FIELD_LEN: usize = 255;

/// The signals a workflow may send with
/// [`WorkflowContext::send_signal`](crate::context::WorkflowContext::send_signal).
///
/// Built from the signal types themselves, never from hand-written names.
/// It keeps a workflow from sending, by mistake, a signal that is not its
/// own; it is not a security boundary: code running in a worker can reach the
/// store and the internal API anyway.
///
/// # Examples
///
/// ```
/// use ironflow_engine::signal::{SentSignals, Signal};
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize, JsonSchema)]
/// struct HumanReplied {
///     body: String,
/// }
///
/// impl Signal for HumanReplied {
///     const NAME: &'static str = "gitlab.human_replied";
/// }
///
/// let sent = SentSignals::new().allow::<HumanReplied>();
/// assert!(sent.allows(HumanReplied::NAME));
/// assert!(!sent.allows("ci.pipeline_finished"));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SentSignals {
    names: Vec<&'static str>,
}

impl SentSignals {
    /// An empty declaration: no signal may be sent.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::signal::SentSignals;
    ///
    /// assert!(SentSignals::new().names().is_empty());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Allow sending `S`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::signal::{SentSignals, Signal};
    /// use schemars::JsonSchema;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// struct Deployed {
    ///     version: String,
    /// }
    ///
    /// impl Signal for Deployed {
    ///     const NAME: &'static str = "deploy.done";
    /// }
    ///
    /// let sent = SentSignals::new().allow::<Deployed>().allow::<Deployed>();
    /// assert_eq!(sent.names(), ["deploy.done"]);
    /// ```
    pub fn allow<S: Signal>(mut self) -> Self {
        if !self.names.contains(&S::NAME) {
            self.names.push(S::NAME);
        }
        self
    }

    /// Whether the signal named `name` may be sent.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::signal::SentSignals;
    ///
    /// assert!(!SentSignals::new().allows("deploy.done"));
    /// ```
    pub fn allows(&self, name: &str) -> bool {
        self.names.contains(&name)
    }

    /// Names of the allowed signals, in declaration order.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::signal::SentSignals;
    ///
    /// let empty: &[&str] = &[];
    /// assert_eq!(SentSignals::new().names(), empty);
    /// ```
    pub fn names(&self) -> &[&'static str] {
        &self.names
    }
}

/// The run and step a signal was sent from, recorded on the
/// [`SignalReceived`](crate::notify::Event::SignalReceived) event.
///
/// # Examples
///
/// ```
/// use ironflow_engine::signal::SignalOrigin;
/// use uuid::Uuid;
///
/// let origin = SignalOrigin { run_id: Uuid::now_v7(), step_id: Uuid::now_v7() };
/// assert_ne!(origin.run_id, origin.step_id);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalOrigin {
    /// The sending run.
    pub run_id: Uuid,
    /// The send step of that run.
    pub step_id: Uuid,
}

/// Future returned by [`SignalSender::send`].
pub type SignalSendFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SignalDelivery, EngineError>> + Send + 'a>>;

/// Where [`WorkflowContext::send_signal`](crate::context::WorkflowContext::send_signal)
/// delivers.
///
/// An engine delivers in its own process by default, through
/// [`Engine::deliver_signal_from`](crate::engine::Engine::deliver_signal_from).
/// A worker, whose store cannot deliver, sets one that goes through the API
/// with [`Engine::with_signal_sender`](crate::engine::Engine::with_signal_sender).
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_engine::engine::Engine;
/// use ironflow_engine::signal::{SignalOrigin, SignalSendFuture, SignalSender};
/// use ironflow_store::models::NewSignal;
///
/// /// Delivers through another engine, sharing the same store.
/// struct Through(Arc<Engine>);
///
/// impl SignalSender for Through {
///     fn send(&self, signal: NewSignal, origin: SignalOrigin) -> SignalSendFuture<'_> {
///         Box::pin(self.0.deliver_signal_from(signal, Some(origin)))
///     }
/// }
/// ```
pub trait SignalSender: Send + Sync {
    /// Deliver `signal`, sent by the step `origin`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidSignal`] for a malformed signal, and
    /// [`EngineError::Store`] when it cannot be stored or delivered.
    fn send(&self, signal: NewSignal, origin: SignalOrigin) -> SignalSendFuture<'_>;
}

/// Key of the payload JSON schema in a signal step's input.
pub const SIGNAL_SCHEMA_KEY: &str = "schema";

/// Key of the timeout flag in a signal step's output.
pub const SIGNAL_TIMED_OUT_KEY: &str = "timed_out";

/// Result of delivering a signal.
///
/// # Examples
///
/// ```
/// use ironflow_engine::signal::SignalDelivery;
/// use uuid::Uuid;
///
/// let delivery = SignalDelivery {
///     signal_id: Uuid::now_v7(),
///     duplicate: false,
///     resumed: Vec::new(),
///     rejected: Vec::new(),
/// };
/// assert!(delivery.resumed.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalDelivery {
    /// ID of the stored signal (the pre-existing one on a duplicate).
    pub signal_id: Uuid,
    /// `true` when the idempotency ID was already used: nothing was delivered.
    pub duplicate: bool,
    /// Waiting steps the signal resolved.
    pub resumed: Vec<SignalResumed>,
    /// Waiting steps whose payload schema the signal did not match. These
    /// steps keep waiting.
    pub rejected: Vec<SignalRejected>,
}

/// A waiting step resolved by a signal.
///
/// # Examples
///
/// ```
/// use ironflow_engine::signal::SignalResumed;
/// use uuid::Uuid;
///
/// let resumed = SignalResumed { run_id: Uuid::now_v7(), step_id: Uuid::now_v7() };
/// assert_ne!(resumed.run_id, resumed.step_id);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalResumed {
    /// Run owning the step.
    pub run_id: Uuid,
    /// The resolved signal step.
    pub step_id: Uuid,
}

/// A waiting step a signal could not resolve.
///
/// # Examples
///
/// ```
/// use ironflow_engine::signal::SignalRejected;
/// use uuid::Uuid;
///
/// let rejected = SignalRejected {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     error: "\"status\" is a required property".to_string(),
/// };
/// assert!(rejected.error.contains("status"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalRejected {
    /// Run owning the step.
    pub run_id: Uuid,
    /// The step that keeps waiting.
    pub step_id: Uuid,
    /// Why the payload was refused.
    pub error: String,
}

/// Output recorded on a signal step resolved by `signal`.
pub(crate) fn received_output(signal: &StoredSignal) -> Value {
    json!({
        SIGNAL_TIMED_OUT_KEY: false,
        "signal_id": signal.id,
        "payload": signal.payload,
    })
}

/// Output recorded on a signal step whose deadline passed.
pub(crate) fn timed_out_output() -> Value {
    json!({ SIGNAL_TIMED_OUT_KEY: true })
}

/// Reject a signal whose name, key or idempotency ID is empty or longer than
/// [`MAX_SIGNAL_FIELD_LEN`].
pub(crate) fn validate_signal(signal: &NewSignal) -> Result<(), EngineError> {
    let fields = [
        ("name", Some(&signal.name)),
        ("key", Some(&signal.key)),
        ("idempotency_id", signal.idempotency_id.as_ref()),
    ];
    for (field, value) in fields {
        let Some(value) = value else { continue };
        if value.trim().is_empty() {
            return Err(EngineError::InvalidSignal(format!(
                "signal {field} must not be empty"
            )));
        }
        if value.chars().count() > MAX_SIGNAL_FIELD_LEN {
            return Err(EngineError::InvalidSignal(format!(
                "signal {field} must be at most {MAX_SIGNAL_FIELD_LEN} characters"
            )));
        }
    }
    Ok(())
}

/// Validate `payload` against a JSON schema, joining every violation.
pub(crate) fn validate_payload(schema: &Value, payload: &Value) -> Result<(), String> {
    let validator = validator_for(schema).map_err(|e| format!("invalid payload schema: {e}"))?;
    let errors: Vec<String> = validator
        .iter_errors(payload)
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Validate `payload` against the schema a signal step stored in its input.
pub(crate) fn validate_step_payload(input: Option<&Value>, payload: &Value) -> Result<(), String> {
    let schema = input
        .and_then(|i| i.get(SIGNAL_SCHEMA_KEY))
        .ok_or_else(|| "step has no stored schema".to_string())?;
    validate_payload(schema, payload)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use schemars::schema_for;
    use serde_json::to_value;

    use super::*;

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct PipelineFinished {
        status: String,
    }

    fn schema() -> Value {
        to_value(schema_for!(PipelineFinished)).unwrap()
    }

    #[test]
    fn validate_payload_accepts_a_matching_payload() {
        assert_eq!(
            validate_payload(&schema(), &json!({"status": "success"})),
            Ok(())
        );
    }

    #[test]
    fn validate_payload_rejects_a_mismatching_payload() {
        let err = validate_payload(&schema(), &json!({"state": 1})).unwrap_err();
        assert!(err.contains("status"), "got {err}");
    }

    #[test]
    fn validate_payload_rejects_an_invalid_schema() {
        let err = validate_payload(&json!({"type": 12}), &json!({})).unwrap_err();
        assert!(err.contains("invalid payload schema"), "got {err}");
    }

    #[test]
    fn validate_step_payload_requires_a_stored_schema() {
        let err = validate_step_payload(None, &json!({})).unwrap_err();
        assert_eq!(err, "step has no stored schema");

        let input = json!({ SIGNAL_SCHEMA_KEY: schema() });
        assert_eq!(
            validate_step_payload(Some(&input), &json!({"status": "ok"})),
            Ok(())
        );
    }

    #[test]
    fn signal_outputs_carry_the_timeout_flag() {
        let signal = StoredSignal {
            id: Uuid::now_v7(),
            name: "ci.done".to_string(),
            key: "abc".to_string(),
            payload: json!({"status": "success"}),
            idempotency_id: None,
            received_at: Utc::now(),
        };
        let received = received_output(&signal);
        assert_eq!(received[SIGNAL_TIMED_OUT_KEY], json!(false));
        assert_eq!(received["payload"], signal.payload);
        assert_eq!(received["signal_id"], json!(signal.id));
        assert_eq!(timed_out_output(), json!({"timed_out": true}));
    }
}
