//! [`DecisionConfig`] -- configuration for a typed machine-decision step.
//!
//! Builds a [`DecisionRequest`] for a [`DecisionProvider`](ironflow_core::decision::DecisionProvider)
//! and carries the escalation threshold used to route low-confidence answers to a
//! human approval gate.

use std::collections::BTreeMap;

use ironflow_core::decision::{DecisionModel, DecisionQuestion, DecisionRequest, NoulCriteria};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The default model route for the System One decision backend.
pub const DEFAULT_DECISION_MODEL: &str = DecisionModel::LATEST;

/// Configuration for a [`decision`](crate::context::WorkflowContext::decision) step.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::DecisionConfig;
///
/// let config = DecisionConfig::new("Payouts have been failing for 3 days")
///     .noul("is_urgent", "Does this convey urgency?")
///     .choice("department", "Which team?", &["billing", "technical", "sales"])
///     .score("frustration", "How frustrated is the customer?", &["Calm", "Frustrated", "Very angry"])
///     .escalate_below(0.7);
///
/// assert_eq!(config.questions.len(), 3);
/// assert_eq!(config.escalate_below, Some(0.7));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionConfig {
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
    /// ```
    pub fn new(state: impl Serialize) -> Self {
        Self {
            state: serde_json::to_value(state).unwrap_or(Value::Null),
            model: DecisionModel::default(),
            questions: BTreeMap::new(),
            escalate_below: None,
        }
    }

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

    /// Add a yes/no question. The answer is the probability of "yes".
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    ///
    /// let config = DecisionConfig::new("x").noul("urgent", "Is this urgent?");
    /// assert!(config.questions.contains_key("urgent"));
    /// ```
    pub fn noul(self, name: &str, instructions: impl Serialize) -> Self {
        self.noul_with(name, instructions, NoulCriteria::default())
    }

    /// Add a yes/no question with explicit true/false criteria.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    /// use ironflow_core::decision::NoulCriteria;
    ///
    /// let config = DecisionConfig::new("x").noul_with(
    ///     "urgent",
    ///     "Is this urgent?",
    ///     NoulCriteria { if_true: Some("time-sensitive".into()), if_false: None },
    /// );
    /// assert!(config.questions.contains_key("urgent"));
    /// ```
    pub fn noul_with(
        mut self,
        name: &str,
        instructions: impl Serialize,
        criteria: NoulCriteria,
    ) -> Self {
        self.questions.insert(
            name.to_string(),
            DecisionQuestion::Noul {
                instructions: to_value(instructions),
                criteria,
            },
        );
        self
    }

    /// Add a selection among named options (no per-option descriptions).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    ///
    /// let config = DecisionConfig::new("x").choice("team", "Which team?", &["billing", "tech"]);
    /// assert!(config.questions.contains_key("team"));
    /// ```
    pub fn choice(self, name: &str, instructions: impl Serialize, options: &[&str]) -> Self {
        let described: Vec<(&str, Option<&str>)> = options.iter().map(|o| (*o, None)).collect();
        self.choice_described(name, instructions, &described)
    }

    /// Add a selection among named options, each with an optional description.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    ///
    /// let config = DecisionConfig::new("x").choice_described(
    ///     "team",
    ///     "Which team?",
    ///     &[("billing", Some("payments")), ("tech", Some("bugs"))],
    /// );
    /// assert!(config.questions.contains_key("team"));
    /// ```
    pub fn choice_described(
        mut self,
        name: &str,
        instructions: impl Serialize,
        options: &[(&str, Option<&str>)],
    ) -> Self {
        let criteria = options
            .iter()
            .map(|(label, desc)| (label.to_string(), desc.map(str::to_string)))
            .collect();
        self.questions.insert(
            name.to_string(),
            DecisionQuestion::Choice {
                instructions: to_value(instructions),
                criteria,
            },
        );
        self
    }

    /// Add a rating against ordered levels (index 0..N-1).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::DecisionConfig;
    ///
    /// let config = DecisionConfig::new("x").score("mood", "How angry?", &["Calm", "Angry"]);
    /// assert!(config.questions.contains_key("mood"));
    /// ```
    pub fn score(mut self, name: &str, instructions: impl Serialize, levels: &[&str]) -> Self {
        self.questions.insert(
            name.to_string(),
            DecisionQuestion::Score {
                instructions: to_value(instructions),
                criteria: levels.iter().map(|l| l.to_string()).collect(),
            },
        );
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
    pub fn to_request(&self) -> DecisionRequest {
        DecisionRequest {
            state: self.state.clone(),
            model: self.model.clone(),
            questions: self.questions.clone(),
        }
    }
}

fn to_value(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_assembles_questions() {
        let config = DecisionConfig::new("state")
            .noul("a", "yes/no?")
            .choice("b", "pick", &["x", "y"])
            .score("c", "rate", &["low", "high"])
            .escalate_below(0.6);
        assert_eq!(config.questions.len(), 3);
        assert_eq!(config.escalate_below, Some(0.6));
        assert_eq!(config.model, "jev-latest");
    }

    #[test]
    fn to_request_carries_state_and_questions() {
        let config = DecisionConfig::new("hello").noul("a", "?");
        let request = config.to_request();
        assert_eq!(request.state, serde_json::json!("hello"));
        assert_eq!(request.questions.len(), 1);
    }

    #[test]
    fn decision_config_serde_roundtrip() {
        let config = DecisionConfig::new("s")
            .noul("a", "?")
            .choice("b", "?", &["x"])
            .escalate_below(0.5);
        let json = serde_json::to_string(&config).unwrap();
        let back: DecisionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.questions.len(), 2);
        assert_eq!(back.escalate_below, Some(0.5));
    }

    #[test]
    fn model_defaults_when_missing_in_json() {
        let config: DecisionConfig = serde_json::from_str(r#"{"state":"s"}"#).unwrap();
        assert_eq!(config.model, "jev-latest");
        assert!(config.questions.is_empty());
    }
}
