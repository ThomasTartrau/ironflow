//! Answer and output types returned by a [`DecisionProvider`](super::DecisionProvider).

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use crate::decision::DecisionModel;
use crate::error::DecisionError;

/// Jev (System One) input price in USD per million tokens.
///
/// Output tokens are unmetered for System One models (no autoregressive decoding),
/// so only input tokens are billed. See <https://typesafe.ai>.
pub const JEV_INPUT_USD_PER_MTOK: f64 = 0.042;

/// The typed answer to a [`DecisionQuestion::Noul`](super::DecisionQuestion::Noul).
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::NoulAnswer;
///
/// let a = NoulAnswer { noul: 0.92 };
/// assert!(a.noul > 0.9);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoulAnswer {
    /// Probability that the answer is "yes", in `[0, 1]`.
    pub noul: f64,
}

/// The typed answer to a [`DecisionQuestion::Choice`](super::DecisionQuestion::Choice).
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::ChoiceAnswer;
/// use std::collections::BTreeMap;
///
/// let a = ChoiceAnswer {
///     choice: "technical".to_string(),
///     probabilities: BTreeMap::from([("technical".to_string(), 0.85)]),
///     confidence: 0.82,
/// };
/// assert_eq!(a.choice, "technical");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceAnswer {
    /// The selected option.
    pub choice: String,
    /// Probability mass over every option.
    #[serde(default)]
    pub probabilities: BTreeMap<String, f64>,
    /// Calibrated confidence in `[0, 1]`, derived from the distribution.
    pub confidence: f64,
}

/// The typed answer to a [`DecisionQuestion::Score`](super::DecisionQuestion::Score).
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::ScoreAnswer;
/// use std::collections::BTreeMap;
///
/// let a = ScoreAnswer {
///     score: 1.6,
///     legend: BTreeMap::from([("2".to_string(), "Very angry".to_string())]),
///     probabilities: BTreeMap::from([("2".to_string(), 0.65)]),
///     confidence: 0.78,
/// };
/// assert!(a.score > 1.5);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreAnswer {
    /// Probability-weighted score across the levels.
    pub score: f64,
    /// Level index (as a string) -> description.
    #[serde(default)]
    pub legend: BTreeMap<String, String>,
    /// Probability mass over each level index.
    #[serde(default)]
    pub probabilities: BTreeMap<String, f64>,
    /// Calibrated confidence in `[0, 1]`, derived from the distribution.
    pub confidence: f64,
}

/// A typed answer to one question, tagged by its kind.
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::{DecisionAnswer, NoulAnswer};
///
/// // A noul answer's confidence is derived: 2 * |p - 0.5|.
/// let a = DecisionAnswer::Noul(NoulAnswer { noul: 0.92 });
/// assert!((a.confidence() - 0.84).abs() < 1e-9);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, IntoStaticStr)]
#[serde(tag = "type", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum DecisionAnswer {
    /// A yes/no probability answer.
    Noul(NoulAnswer),
    /// A selection answer.
    Choice(ChoiceAnswer),
    /// A rating answer.
    Score(ScoreAnswer),
}

impl DecisionAnswer {
    /// The calibrated confidence of this answer, in `[0, 1]`.
    ///
    /// For [`Choice`](DecisionAnswer::Choice) and [`Score`](DecisionAnswer::Score)
    /// this is the provider-reported `confidence`. For [`Noul`](DecisionAnswer::Noul),
    /// which reports only a probability `p`, confidence is derived as `2 * |p - 0.5|`:
    /// `p = 0.5` yields `0` (maximally uncertain), `p = 0` or `p = 1` yields `1`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::{DecisionAnswer, NoulAnswer};
    ///
    /// let coin = DecisionAnswer::Noul(NoulAnswer { noul: 0.5 });
    /// assert_eq!(coin.confidence(), 0.0);
    /// ```
    pub fn confidence(&self) -> f64 {
        match self {
            DecisionAnswer::Noul(a) => 2.0 * (a.noul - 0.5).abs(),
            DecisionAnswer::Choice(a) => a.confidence,
            DecisionAnswer::Score(a) => a.confidence,
        }
    }

    /// The kind name of this answer (`"noul"`, `"choice"`, or `"score"`).
    fn kind(&self) -> &'static str {
        self.into()
    }
}

/// Token usage reported by the provider.
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::DecisionUsage;
///
/// let usage = DecisionUsage { input_tokens: 312, output_tokens: 0 };
/// assert_eq!(usage.input_tokens, 312);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionUsage {
    /// Number of input tokens billed.
    #[serde(default)]
    pub input_tokens: u64,
    /// Number of output tokens (unmetered for System One models).
    #[serde(default)]
    pub output_tokens: u64,
}

impl DecisionUsage {
    /// The USD cost of this usage, imputed from input tokens at the Jev rate
    /// ([`JEV_INPUT_USD_PER_MTOK`]).
    ///
    /// Output tokens are unmetered for System One models, so they do not
    /// contribute to the cost.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::DecisionUsage;
    /// use rust_decimal::Decimal;
    ///
    /// // 1_000_000 input tokens at $0.042 / M = $0.042.
    /// let usage = DecisionUsage { input_tokens: 1_000_000, output_tokens: 0 };
    /// assert_eq!(usage.cost_usd(), Decimal::try_from(0.042).unwrap());
    /// assert_eq!(DecisionUsage::default().cost_usd(), Decimal::ZERO);
    /// ```
    pub fn cost_usd(&self) -> Decimal {
        let usd = self.input_tokens as f64 * JEV_INPUT_USD_PER_MTOK / 1_000_000.0;
        Decimal::try_from(usd).unwrap_or(Decimal::ZERO)
    }
}

/// The typed result of a decision: answers keyed by question name, plus usage.
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::{DecisionOutput, DecisionAnswer, NoulAnswer, DecisionUsage};
/// use std::collections::BTreeMap;
///
/// let output = DecisionOutput {
///     model: Some("jev-latest".into()),
///     answers: BTreeMap::from([
///         ("is_urgent".to_string(), DecisionAnswer::Noul(NoulAnswer { noul: 0.92 })),
///     ]),
///     usage: DecisionUsage::default(),
/// };
/// assert_eq!(output.noul("is_urgent").unwrap(), 0.92);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionOutput {
    /// Model that performed the evaluation, if the provider reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<DecisionModel>,
    /// Answers keyed by the question names from the request.
    pub answers: BTreeMap<String, DecisionAnswer>,
    /// Token usage.
    #[serde(default)]
    pub usage: DecisionUsage,
}

impl DecisionOutput {
    /// Look up an answer by name.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::{DecisionOutput, DecisionAnswer, NoulAnswer, DecisionUsage};
    /// use std::collections::BTreeMap;
    ///
    /// let output = DecisionOutput {
    ///     model: None,
    ///     answers: BTreeMap::from([("q".to_string(), DecisionAnswer::Noul(NoulAnswer { noul: 0.1 }))]),
    ///     usage: DecisionUsage::default(),
    /// };
    /// assert!(output.answer("q").is_some());
    /// assert!(output.answer("missing").is_none());
    /// ```
    pub fn answer(&self, name: &str) -> Option<&DecisionAnswer> {
        self.answers.get(name)
    }

    /// The probability of a [`Noul`](DecisionAnswer::Noul) answer.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::NotFound`] if no answer has that name, or
    /// [`DecisionError::TypeMismatch`] if the answer is not a noul.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::{DecisionOutput, DecisionAnswer, NoulAnswer, DecisionUsage};
    /// use std::collections::BTreeMap;
    ///
    /// let output = DecisionOutput {
    ///     model: None,
    ///     answers: BTreeMap::from([("q".to_string(), DecisionAnswer::Noul(NoulAnswer { noul: 0.7 }))]),
    ///     usage: DecisionUsage::default(),
    /// };
    /// assert_eq!(output.noul("q").unwrap(), 0.7);
    /// ```
    pub fn noul(&self, name: &str) -> Result<f64, DecisionError> {
        match self.require(name)? {
            DecisionAnswer::Noul(a) => Ok(a.noul),
            other => Err(mismatch(name, "noul", other)),
        }
    }

    /// The [`ChoiceAnswer`] for a choice question.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::NotFound`] if no answer has that name, or
    /// [`DecisionError::TypeMismatch`] if the answer is not a choice.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::{DecisionOutput, DecisionAnswer, ChoiceAnswer, DecisionUsage};
    /// use std::collections::BTreeMap;
    ///
    /// let output = DecisionOutput {
    ///     model: None,
    ///     answers: BTreeMap::from([("dept".to_string(), DecisionAnswer::Choice(ChoiceAnswer {
    ///         choice: "billing".to_string(),
    ///         probabilities: BTreeMap::new(),
    ///         confidence: 0.9,
    ///     }))]),
    ///     usage: DecisionUsage::default(),
    /// };
    /// assert_eq!(output.choice("dept").unwrap().choice, "billing");
    /// ```
    pub fn choice(&self, name: &str) -> Result<&ChoiceAnswer, DecisionError> {
        match self.require(name)? {
            DecisionAnswer::Choice(a) => Ok(a),
            other => Err(mismatch(name, "choice", other)),
        }
    }

    /// The [`ScoreAnswer`] for a score question.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::NotFound`] if no answer has that name, or
    /// [`DecisionError::TypeMismatch`] if the answer is not a score.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::{DecisionOutput, DecisionAnswer, ScoreAnswer, DecisionUsage};
    /// use std::collections::BTreeMap;
    ///
    /// let output = DecisionOutput {
    ///     model: None,
    ///     answers: BTreeMap::from([("mood".to_string(), DecisionAnswer::Score(ScoreAnswer {
    ///         score: 1.6,
    ///         legend: BTreeMap::new(),
    ///         probabilities: BTreeMap::new(),
    ///         confidence: 0.78,
    ///     }))]),
    ///     usage: DecisionUsage::default(),
    /// };
    /// assert!(output.score("mood").unwrap().score > 1.5);
    /// ```
    pub fn score(&self, name: &str) -> Result<&ScoreAnswer, DecisionError> {
        match self.require(name)? {
            DecisionAnswer::Score(a) => Ok(a),
            other => Err(mismatch(name, "score", other)),
        }
    }

    /// The lowest confidence across every answer, or `None` when there are no answers.
    ///
    /// Used by the engine to decide escalation: a run escalates when the minimum
    /// confidence falls below the configured threshold.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::decision::{DecisionOutput, DecisionAnswer, NoulAnswer, DecisionUsage};
    /// use std::collections::BTreeMap;
    ///
    /// let output = DecisionOutput {
    ///     model: None,
    ///     answers: BTreeMap::from([("q".to_string(), DecisionAnswer::Noul(NoulAnswer { noul: 0.5 }))]),
    ///     usage: DecisionUsage::default(),
    /// };
    /// assert_eq!(output.min_confidence(), Some(0.0));
    /// ```
    pub fn min_confidence(&self) -> Option<f64> {
        self.answers
            .values()
            .map(DecisionAnswer::confidence)
            .fold(None, |acc, c| Some(acc.map_or(c, |a: f64| a.min(c))))
    }

    fn require(&self, name: &str) -> Result<&DecisionAnswer, DecisionError> {
        self.answers
            .get(name)
            .ok_or_else(|| DecisionError::NotFound(name.to_string()))
    }
}

fn mismatch(name: &str, expected: &'static str, got: &DecisionAnswer) -> DecisionError {
    DecisionError::TypeMismatch {
        name: name.to_string(),
        expected,
        actual: got.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn noul_confidence_is_derived_from_probability() {
        assert_eq!(
            DecisionAnswer::Noul(NoulAnswer { noul: 0.5 }).confidence(),
            0.0
        );
        assert_eq!(
            DecisionAnswer::Noul(NoulAnswer { noul: 1.0 }).confidence(),
            1.0
        );
        assert_eq!(
            DecisionAnswer::Noul(NoulAnswer { noul: 0.0 }).confidence(),
            1.0
        );
        assert!((DecisionAnswer::Noul(NoulAnswer { noul: 0.92 }).confidence() - 0.84).abs() < 1e-9);
    }

    #[test]
    fn choice_and_score_confidence_passthrough() {
        let choice = DecisionAnswer::Choice(ChoiceAnswer {
            choice: "a".to_string(),
            probabilities: BTreeMap::new(),
            confidence: 0.7,
        });
        assert_eq!(choice.confidence(), 0.7);

        let score = DecisionAnswer::Score(ScoreAnswer {
            score: 1.0,
            legend: BTreeMap::new(),
            probabilities: BTreeMap::new(),
            confidence: 0.6,
        });
        assert_eq!(score.confidence(), 0.6);
    }

    #[test]
    fn min_confidence_picks_lowest() {
        let output = DecisionOutput {
            model: None,
            answers: BTreeMap::from([
                (
                    "a".to_string(),
                    DecisionAnswer::Noul(NoulAnswer { noul: 1.0 }),
                ),
                (
                    "b".to_string(),
                    DecisionAnswer::Choice(ChoiceAnswer {
                        choice: "x".to_string(),
                        probabilities: BTreeMap::new(),
                        confidence: 0.3,
                    }),
                ),
            ]),
            usage: DecisionUsage::default(),
        };
        assert_eq!(output.min_confidence(), Some(0.3));
    }

    #[test]
    fn cost_is_zero_for_no_tokens() {
        assert_eq!(DecisionUsage::default().cost_usd(), Decimal::ZERO);
    }

    #[test]
    fn cost_scales_with_input_tokens() {
        // 500_000 tokens = half a million = $0.021.
        let usage = DecisionUsage {
            input_tokens: 500_000,
            output_tokens: 0,
        };
        assert_eq!(usage.cost_usd(), Decimal::try_from(0.021).unwrap());
    }

    #[test]
    fn cost_ignores_output_tokens() {
        let usage = DecisionUsage {
            input_tokens: 0,
            output_tokens: 1_000_000,
        };
        assert_eq!(usage.cost_usd(), Decimal::ZERO);
    }

    #[test]
    fn min_confidence_none_when_empty() {
        let output = DecisionOutput {
            model: None,
            answers: BTreeMap::new(),
            usage: DecisionUsage::default(),
        };
        assert_eq!(output.min_confidence(), None);
    }

    #[test]
    fn accessors_return_typed_errors() {
        let output = DecisionOutput {
            model: None,
            answers: BTreeMap::from([(
                "q".to_string(),
                DecisionAnswer::Noul(NoulAnswer { noul: 0.7 }),
            )]),
            usage: DecisionUsage::default(),
        };
        assert_eq!(output.noul("q").unwrap(), 0.7);
        assert!(matches!(
            output.noul("missing"),
            Err(DecisionError::NotFound(_))
        ));
        assert!(matches!(
            output.choice("q"),
            Err(DecisionError::TypeMismatch {
                expected: "choice",
                actual: "noul",
                ..
            })
        ));
    }

    #[test]
    fn response_deserializes_from_wire_format() {
        let wire = json!({
            "model": "jev-latest",
            "answers": {
                "is_urgent": { "type": "noul", "noul": 0.92 },
                "department": {
                    "type": "choice",
                    "choice": "technical",
                    "probabilities": { "billing": 0.08, "technical": 0.85, "sales": 0.07 },
                    "confidence": 0.82
                },
                "frustration": {
                    "type": "score",
                    "score": 1.6,
                    "legend": { "0": "Calm", "1": "Frustrated", "2": "Very angry" },
                    "probabilities": { "0": 0.05, "1": 0.3, "2": 0.65 },
                    "confidence": 0.78
                }
            },
            "usage": { "input_tokens": 312, "output_tokens": 48 }
        });
        let output: DecisionOutput = serde_json::from_value(wire).unwrap();
        assert_eq!(output.noul("is_urgent").unwrap(), 0.92);
        assert_eq!(output.choice("department").unwrap().choice, "technical");
        assert!((output.score("frustration").unwrap().score - 1.6).abs() < 1e-9);
        assert_eq!(output.usage.input_tokens, 312);
    }
}
