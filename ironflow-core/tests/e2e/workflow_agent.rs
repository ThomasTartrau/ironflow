//! Tests for Agent-centric workflows: chaining, structured output.

use ironflow_core::prelude::*;
use serde_json::json;

use crate::helpers::*;

#[tokio::test]
async fn chained_agents() {
    let (fixtures_dir, _guard) = temp_fixtures_dir("chained");

    // First agent: extract keywords
    let prompt1 = "Extract 3 keywords from: Rust is a systems programming language";
    let config1 = make_config(prompt1, None, None);
    let output1 = make_output(json!("systems, programming, Rust"), 0.002, 500, 20);
    write_fixture(&fixtures_dir, &config1, &output1);

    // Second agent: uses output of first
    let prompt2 = "Write a haiku using these keywords: systems, programming, Rust";
    let config2 = make_config(prompt2, None, None);
    let output2 = make_output(
        json!("Systems forged in Rust\nProgramming with precision\nMemory stands safe"),
        0.003,
        600,
        25,
    );
    write_fixture(&fixtures_dir, &config2, &output2);

    let provider = ironflow_core::providers::record_replay::RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &fixtures_dir,
    );

    let keywords = Agent::new()
        .prompt(prompt1)
        .model(Model::HAIKU)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&provider)
        .await
        .unwrap();

    assert!(keywords.text().contains("Rust"));

    let haiku_prompt = format!("Write a haiku using these keywords: {}", keywords.text());
    let haiku = Agent::new()
        .prompt(&haiku_prompt)
        .model(Model::HAIKU)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&provider)
        .await
        .unwrap();

    assert!(haiku.text().contains("Rust"));

    let mut tracker = WorkflowTracker::new("keyword-haiku");
    tracker.record_agent("extract", &keywords);
    tracker.record_agent("haiku", &haiku);

    assert_eq!(tracker.step_count(), 2);
    assert_eq!(tracker.total_cost_usd(), 0.005);
    assert_eq!(tracker.total_input_tokens(), 1100);
    assert_eq!(tracker.total_output_tokens(), 45);
}

#[derive(Deserialize, JsonSchema, Debug, PartialEq)]
struct CodeReview {
    score: u32,
    summary: String,
    issues: Vec<String>,
}

#[tokio::test]
async fn structured_output() {
    let (fixtures_dir, _guard) = temp_fixtures_dir("structured");

    let diff = Shell::new("echo '+ fn add(a: i32, b: i32) -> i32 { a + b }'")
        .await
        .unwrap();

    let prompt = format!("Review this diff:\n\n{}", diff.stdout());

    let schema = schemars::schema_for!(CodeReview);
    let schema_str = serde_json::to_string(&schema).unwrap();

    let config = make_config(&prompt, None, Some(&schema_str));
    let agent_output = make_output(
        json!({
            "score": 9,
            "summary": "Clean addition function",
            "issues": ["No doc comment"]
        }),
        0.008,
        1200,
        80,
    );
    write_fixture(&fixtures_dir, &config, &agent_output);

    let provider = ironflow_core::providers::record_replay::RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &fixtures_dir,
    );

    let result = Agent::new()
        .prompt(&prompt)
        .model(Model::HAIKU)
        .max_turns(1)
        .max_budget_usd(0.10)
        .output::<CodeReview>()
        .run(&provider)
        .await
        .unwrap();

    let review: CodeReview = result.json().unwrap();
    assert_eq!(review.score, 9);
    assert_eq!(review.summary, "Clean addition function");
    assert_eq!(review.issues, vec!["No doc comment"]);
}
