//! [`DecisionConfig`] -- configuration for a typed machine-decision step.
//!
//! Builds a [`DecisionRequest`] for a [`DecisionProvider`](ironflow_core::decision::DecisionProvider)
//! and carries the escalation threshold used to route low-confidence answers to a
//! human approval gate. The questions come from a struct deriving
//! [`DecisionAnswers`], see [`crate::decision`].

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use ironflow_core::decision::{DecisionModel, DecisionQuestion, DecisionRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::decision::DecisionAnswers;

/// The default model route for the System One decision backend.
pub const DEFAULT_DECISION_MODEL: &str = DecisionModel::LATEST;

/// Marker of a [`DecisionConfig`] whose questions are not set yet: call
/// [`DecisionConfig::answers`] before handing it to a step.
#[derive(Debug, Clone, Copy)]
pub struct NoAnswers;

/// Configuration for a [`decision`](crate::context::WorkflowContext::decision) step.
///
/// `T` is the struct the answers are read into. [`new`](DecisionConfig::new)
/// starts without questions; [`answers`](DecisionConfig::answers) sets them
/// from `T`.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::DecisionConfig;
/// use ironflow_engine::decision::{DecisionAnswers, DecisionChoice};
///
/// #[derive(DecisionChoice)]
/// enum Team {
///     Billing,
///     Technical,
/// }
///
/// #[derive(DecisionAnswers)]
/// struct Triage {
///     #[noul("Does this convey urgency?")]
///     is_urgent: f64,
///     #[choice("Which team?")]
///     team: Team,
/// }
///
/// let config = DecisionConfig::new("Payouts have been failing for 3 days")
///     .answers::<Triage>()
///     .escalate_below(0.7);
///
/// assert_eq!(config.questions.len(), 2);
/// assert_eq!(config.escalate_below, Some(0.7));
/// ```
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub struct DecisionConfig<T = NoAnswers> {
    /// The state (content) to evaluate.
    pub state: Value,
    /// Model route (defaults to [`DEFAULT_DECISION_MODEL`]).
    #[serde(default)]
    pub model: DecisionModel,
    /// Typed questions keyed by name.
    #[serde(default)]
    pub questions: BTreeMap<String, DecisionQuestion>,
    /// Escalate to a human approval gate when any answer's confidence falls
    /// below this threshold. `None` never escalates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalate_below: Option<f64>,
    #[serde(skip)]
    answers: PhantomData<fn() -> T>,
}

// Written by hand: a derive would require `T: Debug` / `T: Clone` for a type
// that is never stored.
impl<T> fmt::Debug for DecisionConfig<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecisionConfig")
            .field("state", &self.state)
            .field("model", &self.model)
            .field("questions", &self.questions)
            .field("escalate_below", &self.escalate_below)
            .finish()
    }
}

impl<T> Clone for DecisionConfig<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            model: self.model.clone(),
            questions: self.questions.clone(),
            escalate_below: self.escalate_below,
            answers: PhantomData,
        }
    }
}

impl DecisionConfig {
    /// Create a config for the given state (any serializable value).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    /// use serde_json::json;
    ///
    /// let config = DecisionConfig::new(json!({ "ticket": "outage", "priority": 1 }));
    /// assert_eq!(config.model, "jev-latest");
    /// assert!(config.questions.is_empty());
    /// ```
    pub fn new(state: impl Serialize) -> Self {
        Self {
            state: serde_json::to_value(state).unwrap_or(Value::Null),
            model: DecisionModel::default(),
            questions: BTreeMap::new(),
            escalate_below: None,
            answers: PhantomData,
        }
    }

    /// Ask the questions declared by `T` and read the answers into a `T`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    /// use ironflow_engine::decision::DecisionAnswers;
    ///
    /// #[derive(DecisionAnswers)]
    /// struct Urgency {
    ///     #[noul("Is this urgent?")]
    ///     urgent: f64,
    /// }
    ///
    /// let config = DecisionConfig::new("x").answers::<Urgency>();
    /// assert!(config.questions.contains_key("urgent"));
    /// ```
    pub fn answers<T: DecisionAnswers>(self) -> DecisionConfig<T> {
        DecisionConfig {
            state: self.state,
            model: self.model,
            questions: T::questions(),
            escalate_below: self.escalate_below,
            answers: PhantomData,
        }
    }
}

impl<T> DecisionConfig<T> {
    /// Override the model route.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    ///
    /// let config = DecisionConfig::new("x").model("jev-2");
    /// assert_eq!(config.model, "jev-2");
    /// ```
    pub fn model(mut self, model: impl Into<DecisionModel>) -> Self {
        self.model = model.into();
        self
    }

    /// Set the confidence threshold below which the run escalates to a human.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    ///
    /// let config = DecisionConfig::new("x").escalate_below(0.8);
    /// assert_eq!(config.escalate_below, Some(0.8));
    /// ```
    pub fn escalate_below(mut self, threshold: f64) -> Self {
        self.escalate_below = Some(threshold);
        self
    }

    /// Build the [`DecisionRequest`] sent to the provider.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    ///
    /// let request = DecisionConfig::new("hello").to_request();
    /// assert_eq!(request.state, serde_json::json!("hello"));
    /// ```
    pub fn to_request(&self) -> DecisionRequest {
        DecisionRequest {
            state: self.state.clone(),
            model: self.model.clone(),
            questions: self.questions.clone(),
        }
    }

    /// The same config, without the answer type. This is the form stored as
    /// the step input.
    pub(crate) fn erase(self) -> DecisionConfig {
        DecisionConfig {
            state: self.state,
            model: self.model,
            questions: self.questions,
            escalate_below: self.escalate_below,
            answers: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::{DecisionAnswers, DecisionChoice};

    #[derive(DecisionChoice)]
    enum Pick {
        X,
        Y,
    }

    #[derive(DecisionAnswers)]
    #[allow(dead_code)]
    struct Probe {
        #[noul("yes/no?")]
        a: f64,
        #[choice("pick")]
        b: Pick,
        #[score("rate", levels = ["low", "high"])]
        c: f64,
    }

    #[test]
    fn answers_assembles_the_questions_of_the_type() {
        let config = DecisionConfig::new("state")
            .escalate_below(0.6)
            .answers::<Probe>();
        assert_eq!(config.questions.len(), 3);
        assert_eq!(config.escalate_below, Some(0.6));
        assert_eq!(config.model, "jev-latest");
    }

    #[test]
    fn builders_keep_working_after_answers() {
        let config = DecisionConfig::new("state")
            .answers::<Probe>()
            .model("jev-2")
            .escalate_below(0.5);
        assert_eq!(config.model, "jev-2");
        assert_eq!(config.escalate_below, Some(0.5));
    }

    #[test]
    fn to_request_carries_state_and_questions() {
        let request = DecisionConfig::new("hello").answers::<Probe>().to_request();
        assert_eq!(request.state, serde_json::json!("hello"));
        assert_eq!(request.questions.len(), 3);
    }

    #[test]
    fn erase_keeps_everything_but_the_type() {
        let typed = DecisionConfig::new("s")
            .answers::<Probe>()
            .escalate_below(0.4);
        let questions = typed.questions.clone();
        let erased = typed.erase();
        assert_eq!(erased.questions, questions);
        assert_eq!(erased.escalate_below, Some(0.4));
    }

    #[test]
    fn decision_config_serde_roundtrip() {
        let config = DecisionConfig::new("s")
            .answers::<Probe>()
            .escalate_below(0.5);
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("answers"));
        let back: DecisionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.questions.len(), 3);
        assert_eq!(back.escalate_below, Some(0.5));
    }

    #[test]
    fn model_defaults_when_missing_in_json() {
        let config: DecisionConfig = serde_json::from_str(r#"{"state":"s"}"#).unwrap();
        assert_eq!(config.model, "jev-latest");
        assert!(config.questions.is_empty());
    }
}
