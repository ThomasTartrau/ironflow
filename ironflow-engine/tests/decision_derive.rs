//! Tests for `#[derive(DecisionAnswers)]` and `#[derive(DecisionChoice)]`.
//!
//! The derives turn a struct into the questions of a decision step and read the
//! provider's answers back into it. The answers here are the exact shape the
//! System One provider returns.

use std::collections::BTreeMap;

use serde_json::json;

use ironflow_core::decision::{
    ChoiceAnswer, DecisionAnswer, DecisionOutput, DecisionQuestion, DecisionUsage, NoulAnswer,
    NoulCriteria, ScoreAnswer,
};
use ironflow_core::error::DecisionError;
use ironflow_engine::decision::{DecisionAnswers, DecisionChoice};

#[derive(Debug, Clone, Copy, PartialEq, DecisionChoice)]
enum Team {
    /// Doc comments stay developer documentation, never model criteria.
    #[choice(description = "Payments, invoices and refunds")]
    Billing,
    Technical,
    #[choice(rename = "sales-team")]
    Sales,
    OnCallSRE,
}

#[derive(Debug, PartialEq, DecisionAnswers)]
struct Triage {
    #[noul("Does this convey urgency?")]
    is_urgent: f64,
    #[noul(
        "Is it a refund request?",
        if_true = "Asks for money back",
        if_false = "Anything else"
    )]
    refund: f64,
    #[choice("Which team should handle this?")]
    team: Team,
    #[score(
        "How frustrated is the customer?",
        levels = ["Calm", "Frustrated", "Very angry"]
    )]
    mood: f64,
}

fn choice(label: &str) -> DecisionAnswer {
    DecisionAnswer::Choice(ChoiceAnswer {
        choice: label.to_string(),
        probabilities: BTreeMap::new(),
        confidence: 0.9,
    })
}

fn output(answers: Vec<(&str, DecisionAnswer)>) -> DecisionOutput {
    DecisionOutput {
        model: None,
        answers: answers
            .into_iter()
            .map(|(name, answer)| (name.to_string(), answer))
            .collect(),
        usage: DecisionUsage::default(),
    }
}

fn full_output(team: &str) -> DecisionOutput {
    output(vec![
        ("is_urgent", DecisionAnswer::Noul(NoulAnswer { noul: 0.92 })),
        ("refund", DecisionAnswer::Noul(NoulAnswer { noul: 0.1 })),
        ("team", choice(team)),
        (
            "mood",
            DecisionAnswer::Score(ScoreAnswer {
                score: 1.6,
                legend: BTreeMap::new(),
                probabilities: BTreeMap::new(),
                confidence: 0.8,
            }),
        ),
    ])
}

#[test]
fn choice_options_are_the_variants_in_snake_case() {
    assert_eq!(
        Team::options(),
        vec![
            ("billing", Some("Payments, invoices and refunds")),
            ("technical", None),
            ("sales-team", None),
            ("on_call_sre", None),
        ]
    );
}

#[test]
fn choice_labels_roundtrip() {
    for team in [Team::Billing, Team::Technical, Team::Sales, Team::OnCallSRE] {
        assert_eq!(Team::from_label(team.label()), Some(team));
    }
    assert_eq!(Team::Sales.label(), "sales-team");
    assert_eq!(Team::from_label("Sales"), None);
    assert_eq!(Team::from_label(""), None);
}

#[test]
fn questions_are_the_fields() {
    let questions = Triage::questions();

    let names: Vec<&str> = questions.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["is_urgent", "mood", "refund", "team"]);

    assert_eq!(
        questions["is_urgent"],
        DecisionQuestion::Noul {
            instructions: json!("Does this convey urgency?"),
            criteria: NoulCriteria::default(),
        }
    );
    assert_eq!(
        questions["refund"],
        DecisionQuestion::Noul {
            instructions: json!("Is it a refund request?"),
            criteria: NoulCriteria {
                if_true: Some("Asks for money back".to_string()),
                if_false: Some("Anything else".to_string()),
            },
        }
    );
    assert_eq!(
        questions["team"],
        DecisionQuestion::Choice {
            instructions: json!("Which team should handle this?"),
            criteria: BTreeMap::from([
                (
                    "billing".to_string(),
                    Some("Payments, invoices and refunds".to_string())
                ),
                ("technical".to_string(), None),
                ("sales-team".to_string(), None),
                ("on_call_sre".to_string(), None),
            ]),
        }
    );
    assert_eq!(
        questions["mood"],
        DecisionQuestion::Score {
            instructions: json!("How frustrated is the customer?"),
            criteria: vec![
                "Calm".to_string(),
                "Frustrated".to_string(),
                "Very angry".to_string()
            ],
        }
    );
}

#[test]
fn answers_are_read_back_into_the_struct() {
    let triage = Triage::from_output(&full_output("sales-team")).expect("every answer present");

    assert_eq!(
        triage,
        Triage {
            is_urgent: 0.92,
            refund: 0.1,
            team: Team::Sales,
            mood: 1.6,
        }
    );
}

#[test]
fn a_missing_answer_is_an_error() {
    let mut out = full_output("billing");
    out.answers.remove("mood");

    let err = Triage::from_output(&out).expect_err("mood is missing");

    assert!(matches!(err, DecisionError::NotFound(ref name) if name == "mood"));
}

#[test]
fn an_answer_of_another_kind_is_an_error() {
    let mut out = full_output("billing");
    out.answers.insert(
        "team".to_string(),
        DecisionAnswer::Noul(NoulAnswer { noul: 0.5 }),
    );

    let err = Triage::from_output(&out).expect_err("team is not a choice");

    assert!(matches!(
        err,
        DecisionError::TypeMismatch { ref name, expected: "choice", actual: "noul" } if name == "team"
    ));
}

#[test]
fn an_option_that_is_not_a_variant_is_an_error() {
    let err = Triage::from_output(&full_output("marketing")).expect_err("not a Team");

    assert!(matches!(
        err,
        DecisionError::UnknownChoice { ref name, ref choice } if name == "team" && choice == "marketing"
    ));
}
