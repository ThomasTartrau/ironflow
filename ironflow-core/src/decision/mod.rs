//! [`DecisionProvider`] -- typed, calibrated machine decisions.
//!
//! Where [`AgentProvider`](crate::provider::AgentProvider) drives a conversational
//! LLM (a single prompt, opaque text or JSON out), a [`DecisionProvider`] answers a
//! *map of typed questions* about a *state* and returns *typed answers* with a
//! calibrated confidence. This is the shape of TypeSafe AI's System One model
//! (`Jev`): classify, route, score, yes/no -- fast and cheap, with a probability
//! distribution instead of free text.
//!
//! The trait lives beside [`AgentProvider`](crate::provider::AgentProvider) rather
//! than inside it: Jev has no single prompt, no tools, and no streaming, so forcing
//! it into the agent mould would leak abstraction. A run wires a decision provider
//! independently of its agent provider.
//!
//! # Examples
//!
//! ```
//! use ironflow_core::decision::{DecisionRequest, DecisionQuestion, NoulCriteria};
//! use std::collections::BTreeMap;
//! use serde_json::json;
//!
//! let mut questions = BTreeMap::new();
//! questions.insert(
//!     "is_urgent".to_string(),
//!     DecisionQuestion::Noul {
//!         instructions: json!("Does this convey urgency?"),
//!         criteria: NoulCriteria::default(),
//!     },
//! );
//! let request = DecisionRequest {
//!     state: json!("Help! My payouts have been failing for 3 days."),
//!     model: "jev-latest".into(),
//!     questions,
//! };
//! assert_eq!(request.model, "jev-latest");
//! ```

mod output;
mod request;

use std::future::Future;
use std::pin::Pin;

use crate::error::AgentError;

pub use output::{
    ChoiceAnswer, DecisionAnswer, DecisionOutput, DecisionUsage, JEV_INPUT_USD_PER_MTOK,
    NoulAnswer, ScoreAnswer,
};
pub use request::{DecisionModel, DecisionQuestion, DecisionRequest, NoulCriteria};

/// Boxed future returned by [`DecisionProvider::decide`].
pub type DecideFuture<'a> =
    Pin<Box<dyn Future<Output = Result<DecisionOutput, AgentError>> + Send + 'a>>;

/// A backend that answers a [`DecisionRequest`] with typed, calibrated answers.
///
/// Implement this to plug in any System One decision backend. The built-in
/// `TypeSafeProvider` (feature `provider-typesafe`) speaks the Jev HTTP API;
/// [`RecordReplayDecisionProvider`](crate::providers::record_replay_decision::RecordReplayDecisionProvider)
/// replays captured fixtures for deterministic tests.
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::{DecideFuture, DecisionProvider, DecisionRequest, DecisionOutput, DecisionUsage};
/// use std::collections::BTreeMap;
///
/// struct AlwaysEmpty;
/// impl DecisionProvider for AlwaysEmpty {
///     fn decide<'a>(&'a self, _request: &'a DecisionRequest) -> DecideFuture<'a> {
///         Box::pin(async {
///             Ok(DecisionOutput { model: None, answers: BTreeMap::new(), usage: DecisionUsage::default() })
///         })
///     }
/// }
/// ```
pub trait DecisionProvider: Send + Sync {
    /// Evaluate a decision request and return typed answers.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError`] if the backend fails, times out, rate-limits, or
    /// returns a response that cannot be parsed.
    fn decide<'a>(&'a self, request: &'a DecisionRequest) -> DecideFuture<'a>;
}
