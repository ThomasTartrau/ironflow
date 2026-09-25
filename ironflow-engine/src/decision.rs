//! Typed decisions: questions declared as a struct, answers read back into it.
//!
//! [`DecisionAnswers`] turns a struct into the questions of a
//! [`decision`](crate::context::WorkflowContext::decision) step, one question
//! per field, and reads the provider's answers back into it.
//! [`DecisionChoice`] lists the options of a choice question from the unit
//! variants of an enum. Both are derived:
//!
//! ```no_run
//! use ironflow_engine::config::DecisionConfig;
//! use ironflow_engine::context::WorkflowContext;
//! use ironflow_engine::decision::{DecisionAnswers, DecisionChoice};
//! use ironflow_engine::error::EngineError;
//!
//! #[derive(Debug, DecisionChoice)]
//! enum Team {
//!     #[choice(description = "Payments and invoices")]
//!     Billing,
//!     Technical,
//!     Sales,
//! }
//!
//! #[derive(Debug, DecisionAnswers)]
//! struct Triage {
//!     #[noul("Does this convey urgency?")]
//!     is_urgent: f64,
//!     #[choice("Which team should handle this?")]
//!     team: Team,
//!     #[score("How frustrated is the customer?", levels = ["Calm", "Frustrated", "Very angry"])]
//!     mood: f64,
//! }
//!
//! # async fn example(ctx: &mut WorkflowContext, ticket: &str) -> Result<(), EngineError> {
//! let triage = ctx
//!     .decision("triage", DecisionConfig::new(ticket).answers::<Triage>().escalate_below(0.7))
//!     .await?;
//! match triage.team {
//!     Team::Billing => { /* .. */ }
//!     Team::Technical | Team::Sales => { /* .. */ }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Field attributes of `#[derive(DecisionAnswers)]`
//!
//! Every field is one question, named after the field, with exactly one of:
//!
//! | Attribute | Field type | Answer |
//! |-----------|------------|--------|
//! | `#[noul("..")]`, optionally `if_true = ".."`, `if_false = ".."` | `f64` | Probability of "yes", in `[0, 1]` |
//! | `#[choice("..")]` | an enum deriving [`DecisionChoice`] | The option picked |
//! | `#[score("..", levels = ["..", ".."])]` | `f64` | Probability-weighted level index |
//!
//! The first argument is the instruction sent to the model. A field without a
//! question does not compile:
//!
//! ```compile_fail
//! use ironflow_engine::decision::DecisionAnswers;
//!
//! #[derive(DecisionAnswers)]
//! struct Triage {
//!     is_urgent: f64,
//! }
//! ```
//!
//! nor does a score without levels:
//!
//! ```compile_fail
//! use ironflow_engine::decision::DecisionAnswers;
//!
//! #[derive(DecisionAnswers)]
//! struct Triage {
//!     #[score("How frustrated?")]
//!     mood: f64,
//! }
//! ```
//!
//! nor a field whose type does not fit its question:
//!
//! ```compile_fail,E0308
//! use ironflow_engine::decision::DecisionAnswers;
//!
//! #[derive(DecisionAnswers)]
//! struct Triage {
//!     #[noul("Does this convey urgency?")]
//!     is_urgent: String,
//! }
//! ```
//!
//! # Variant attributes of `#[derive(DecisionChoice)]`
//!
//! An option is labelled with its variant name in `snake_case` (`OnCall` is
//! `on_call`). `#[choice(rename = "..")]` overrides the label,
//! `#[choice(description = "..")]` tells the model what the option means. Doc
//! comments are never sent to the model. Options are unit variants:
//!
//! ```compile_fail
//! use ironflow_engine::decision::DecisionChoice;
//!
//! #[derive(DecisionChoice)]
//! enum Team {
//!     Billing,
//!     Other(String),
//! }
//! ```

use std::collections::BTreeMap;

use ironflow_core::decision::{DecisionOutput, DecisionQuestion};
use ironflow_core::error::DecisionError;

pub use ironflow_engine_macros::{DecisionAnswers, DecisionChoice};

/// A struct whose fields are the questions of a decision step and whose
/// values are the answers.
///
/// Derive it, see the [module documentation](self).
///
/// # Examples
///
/// ```
/// use ironflow_engine::decision::DecisionAnswers;
///
/// #[derive(DecisionAnswers)]
/// struct Review {
///     #[noul("Is the change safe to ship?")]
///     safe: f64,
/// }
///
/// assert!(Review::questions().contains_key("safe"));
/// ```
pub trait DecisionAnswers: Sized {
    /// The questions, keyed by field name.
    fn questions() -> BTreeMap<String, DecisionQuestion>;

    /// Read the provider's answers back into the struct.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::NotFound`] when an answer is missing,
    /// [`DecisionError::TypeMismatch`] when it is of another kind than its
    /// question, and [`DecisionError::UnknownChoice`] when a choice is not one
    /// of the options.
    fn from_output(output: &DecisionOutput) -> Result<Self, DecisionError>;
}

/// An enum whose unit variants are the options of a choice question.
///
/// Derive it, see the [module documentation](self).
///
/// # Examples
///
/// ```
/// use ironflow_engine::decision::DecisionChoice;
///
/// #[derive(Debug, PartialEq, DecisionChoice)]
/// enum Severity {
///     Low,
///     #[choice(description = "Customers are affected")]
///     High,
/// }
///
/// assert_eq!(Severity::options(), vec![("low", None), ("high", Some("Customers are affected"))]);
/// assert_eq!(Severity::from_label("high"), Some(Severity::High));
/// assert_eq!(Severity::Low.label(), "low");
/// ```
pub trait DecisionChoice: Sized {
    /// Every option label with its optional description, in declaration order.
    fn options() -> Vec<(&'static str, Option<&'static str>)>;

    /// The variant labelled `label`, if any.
    fn from_label(label: &str) -> Option<Self>;

    /// The label of this variant.
    fn label(&self) -> &'static str;
}

/// Support code for the derives. Not a public API.
#[doc(hidden)]
pub mod __private {
    use std::collections::BTreeMap;

    use ironflow_core::decision::NoulCriteria;
    use serde_json::Value;

    pub use ironflow_core::decision::{DecisionOutput, DecisionQuestion};
    pub use ironflow_core::error::DecisionError;

    use super::DecisionChoice;

    /// Questions keyed by name.
    pub type Questions = BTreeMap<String, DecisionQuestion>;

    pub fn noul(
        instructions: &str,
        if_true: Option<&str>,
        if_false: Option<&str>,
    ) -> DecisionQuestion {
        DecisionQuestion::Noul {
            instructions: Value::String(instructions.to_string()),
            criteria: NoulCriteria {
                if_true: if_true.map(str::to_string),
                if_false: if_false.map(str::to_string),
            },
        }
    }

    pub fn choice<C: DecisionChoice>(instructions: &str) -> DecisionQuestion {
        DecisionQuestion::Choice {
            instructions: Value::String(instructions.to_string()),
            criteria: C::options()
                .into_iter()
                .map(|(label, description)| (label.to_string(), description.map(str::to_string)))
                .collect(),
        }
    }

    pub fn score(instructions: &str, levels: &[&str]) -> DecisionQuestion {
        DecisionQuestion::Score {
            instructions: Value::String(instructions.to_string()),
            criteria: levels.iter().map(|level| level.to_string()).collect(),
        }
    }

    pub fn read_noul(output: &DecisionOutput, name: &str) -> Result<f64, DecisionError> {
        output.noul(name)
    }

    pub fn read_choice<C: DecisionChoice>(
        output: &DecisionOutput,
        name: &str,
    ) -> Result<C, DecisionError> {
        let picked = &output.choice(name)?.choice;
        C::from_label(picked).ok_or_else(|| DecisionError::UnknownChoice {
            name: name.to_string(),
            choice: picked.clone(),
        })
    }

    pub fn read_score(output: &DecisionOutput, name: &str) -> Result<f64, DecisionError> {
        Ok(output.score(name)?.score)
    }
}
