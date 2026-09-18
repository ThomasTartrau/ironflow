//! Request types for a [`DecisionProvider`](super::DecisionProvider): the state
//! to evaluate and the map of typed questions to ask about it.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A model route for a [`DecisionRequest`], e.g. the System One `jev-latest` alias.
///
/// A thin newtype over the route string: it keeps model identifiers distinct from
/// arbitrary strings at the type level while still accepting any custom route a
/// backend exposes. Serializes transparently as the bare string, so the wire
/// format is unchanged.
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::DecisionModel;
///
/// assert_eq!(DecisionModel::default().as_str(), "jev-latest");
/// assert_eq!(DecisionModel::from("jev-2").as_str(), "jev-2");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DecisionModel(String);

impl DecisionModel {
    /// The default early-access route (`jev-latest`).
    pub const LATEST: &'static str = "jev-latest";

    /// Wrap a model route.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::DecisionModel;
    ///
    /// let model = DecisionModel::new("jev-2");
    /// assert_eq!(model.as_str(), "jev-2");
    /// ```
    pub fn new(route: impl Into<String>) -> Self {
        Self(route.into())
    }

    /// The route as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::DecisionModel;
    ///
    /// assert_eq!(DecisionModel::from("jev-latest").as_str(), "jev-latest");
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for DecisionModel {
    fn default() -> Self {
        Self(Self::LATEST.to_string())
    }
}

impl From<&str> for DecisionModel {
    fn from(route: &str) -> Self {
        Self(route.to_string())
    }
}

impl From<String> for DecisionModel {
    fn from(route: String) -> Self {
        Self(route)
    }
}

impl fmt::Display for DecisionModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for DecisionModel {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for DecisionModel {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for DecisionModel {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Optional natural-language criteria for a [`DecisionQuestion::Noul`] question.
///
/// Both sides are optional: an empty [`NoulCriteria`] asks the model to decide
/// with the instructions alone.
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::NoulCriteria;
///
/// let criteria = NoulCriteria {
///     if_true: Some("Explicitly time-sensitive".to_string()),
///     if_false: Some("No urgency expressed".to_string()),
/// };
/// assert!(!criteria.is_empty());
/// assert!(NoulCriteria::default().is_empty());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoulCriteria {
    /// Description of what a "yes" (true) looks like.
    #[serde(rename = "true", default, skip_serializing_if = "Option::is_none")]
    pub if_true: Option<String>,
    /// Description of what a "no" (false) looks like.
    #[serde(rename = "false", default, skip_serializing_if = "Option::is_none")]
    pub if_false: Option<String>,
}

impl NoulCriteria {
    /// Whether both criteria are unset.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::NoulCriteria;
    ///
    /// assert!(NoulCriteria::default().is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.if_true.is_none() && self.if_false.is_none()
    }
}

/// A single typed question in a [`DecisionRequest`].
///
/// The three variants mirror the System One question types. `instructions` is a
/// free-form [`Value`] (string, object, or array) describing what to evaluate.
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::DecisionQuestion;
/// use serde_json::json;
///
/// let q = DecisionQuestion::Score {
///     instructions: json!("How frustrated is the customer?"),
///     criteria: vec!["Calm".into(), "Frustrated".into(), "Very angry".into()],
/// };
/// let wire = serde_json::to_value(&q).unwrap();
/// assert_eq!(wire["type"], "score");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionQuestion {
    /// A yes/no question. The answer is the probability that the answer is "yes".
    Noul {
        /// What to evaluate (string, object, or array).
        instructions: Value,
        /// Optional descriptions of the true/false sides.
        #[serde(default, skip_serializing_if = "NoulCriteria::is_empty")]
        criteria: NoulCriteria,
    },
    /// A selection among named options. `criteria` maps each option to an
    /// optional description.
    Choice {
        /// What to evaluate (string, object, or array).
        instructions: Value,
        /// Options: name -> optional description.
        criteria: BTreeMap<String, Option<String>>,
    },
    /// A rating against ordered, descriptive levels (index 0..N-1).
    Score {
        /// What to evaluate (string, object, or array).
        instructions: Value,
        /// Ordered level descriptions.
        criteria: Vec<String>,
    },
}

/// A request to a [`DecisionProvider`](super::DecisionProvider): a state plus a
/// map of typed questions.
///
/// Answers come back under the same keys used in [`questions`](Self::questions).
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::{DecisionRequest, DecisionQuestion};
/// use std::collections::BTreeMap;
/// use serde_json::json;
///
/// let mut questions = BTreeMap::new();
/// questions.insert(
///     "department".to_string(),
///     DecisionQuestion::Choice {
///         instructions: json!("Which team should handle this?"),
///         criteria: BTreeMap::from([
///             ("billing".to_string(), Some("Payments".to_string())),
///             ("technical".to_string(), Some("Bugs".to_string())),
///         ]),
///     },
/// );
/// let request = DecisionRequest { state: json!("outage"), model: "jev-latest".into(), questions };
/// assert_eq!(request.questions.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRequest {
    /// The content to evaluate: a string, or structured JSON.
    pub state: Value,
    /// Model route (e.g. [`DecisionModel::LATEST`]).
    pub model: DecisionModel,
    /// Typed questions keyed by a name the caller chooses.
    pub questions: BTreeMap<String, DecisionQuestion>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn noul_criteria_empty() {
        assert!(NoulCriteria::default().is_empty());
        assert!(
            !NoulCriteria {
                if_true: Some("x".into()),
                if_false: None,
            }
            .is_empty()
        );
    }

    #[test]
    fn request_serializes_to_wire_format() {
        let mut questions = BTreeMap::new();
        questions.insert(
            "is_urgent".to_string(),
            DecisionQuestion::Noul {
                instructions: json!("Does this convey urgency?"),
                criteria: NoulCriteria {
                    if_true: Some("time-sensitive".to_string()),
                    if_false: None,
                },
            },
        );
        let request = DecisionRequest {
            state: json!("outage"),
            model: "jev-latest".into(),
            questions,
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["questions"]["is_urgent"]["type"], "noul");
        assert_eq!(
            wire["questions"]["is_urgent"]["criteria"]["true"],
            "time-sensitive"
        );
        assert!(
            wire["questions"]["is_urgent"]["criteria"]
                .get("false")
                .is_none()
        );
    }

    #[test]
    fn choice_and_score_roundtrip() {
        let q = DecisionQuestion::Choice {
            instructions: json!("team?"),
            criteria: BTreeMap::from([("billing".to_string(), Some("pay".to_string()))]),
        };
        let back: DecisionQuestion =
            serde_json::from_value(serde_json::to_value(&q).unwrap()).unwrap();
        assert_eq!(q, back);

        let q = DecisionQuestion::Score {
            instructions: json!("mood?"),
            criteria: vec!["Calm".into(), "Angry".into()],
        };
        let back: DecisionQuestion =
            serde_json::from_value(serde_json::to_value(&q).unwrap()).unwrap();
        assert_eq!(q, back);
    }
}
