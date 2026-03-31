//! Tests for replay determinism and record/replay infrastructure.

use ironflow_core::prelude::*;
use ironflow_core::providers::record_replay::hash_config;
use serde_json::json;

use crate::helpers::*;

#[tokio::test]
async fn replay_is_deterministic() {
    let (fixtures_dir, _guard) = temp_fixtures_dir("deterministic");

    let prompt = "What is 2+2?";
    let config = make_config(prompt, None, None);
    let agent_output = make_output(json!("4"), 0.001, 100, 5);
    write_fixture(&fixtures_dir, &config, &agent_output);

    let provider = ironflow_core::providers::record_replay::RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &fixtures_dir,
    );

    for _ in 0..3 {
        let result = Agent::new()
            .prompt(prompt)
            .model(Model::HAIKU)
            .max_turns(1)
            .max_budget_usd(0.10)
            .run(&provider)
            .await
            .unwrap();

        assert_eq!(result.text(), "4");
        assert_eq!(result.cost_usd(), Some(0.001));
    }
}

#[tokio::test]
async fn different_prompts_produce_different_hashes() {
    let config_a = make_config("prompt A", None, None);
    let config_b = make_config("prompt B", None, None);

    let hash_a = hash_config(&config_a);
    let hash_b = hash_config(&config_b);

    assert_ne!(hash_a, hash_b);
}

#[tokio::test]
async fn same_prompt_produces_same_hash() {
    let config1 = make_config("identical prompt", Some("system"), None);
    let config2 = make_config("identical prompt", Some("system"), None);

    let hash1 = hash_config(&config1);
    let hash2 = hash_config(&config2);

    assert_eq!(hash1, hash2);
}

#[tokio::test]
async fn system_prompt_affects_hash() {
    let config_a = make_config("same prompt", Some("system A"), None);
    let config_b = make_config("same prompt", Some("system B"), None);

    let hash_a = hash_config(&config_a);
    let hash_b = hash_config(&config_b);

    assert_ne!(hash_a, hash_b);
}

#[tokio::test]
async fn json_schema_affects_hash() {
    let config_a = make_config("same prompt", None, Some(r#"{"type":"object"}"#));
    let config_b = make_config("same prompt", None, Some(r#"{"type":"array"}"#));

    let hash_a = hash_config(&config_a);
    let hash_b = hash_config(&config_b);

    assert_ne!(hash_a, hash_b);
}

#[tokio::test]
async fn record_mode_writes_fixture_file() {
    let (fixtures_dir, _guard) = temp_fixtures_dir("record-mode");

    let prompt = "record test";
    let config = make_config(prompt, None, None);
    let expected_path =
        std::path::PathBuf::from(&fixtures_dir).join(format!("{}.json", hash_config(&config)));

    // Pre-write a fixture so the inner provider can replay it
    // (we test record mode by wrapping a replay provider)
    let inner_dir = format!("{}-inner", &fixtures_dir);
    let inner_output = make_output(json!("recorded answer"), 0.002, 200, 10);
    write_fixture(&inner_dir, &config, &inner_output);
    let _inner_guard = FixtureGuard::new(&inner_dir);

    let inner = ironflow_core::providers::record_replay::RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &inner_dir,
    );
    let recorder =
        ironflow_core::providers::record_replay::RecordReplayProvider::record(inner, &fixtures_dir);

    let result = Agent::new()
        .prompt(prompt)
        .model(Model::HAIKU)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&recorder)
        .await
        .unwrap();

    assert_eq!(result.text(), "recorded answer");
    assert!(
        expected_path.exists(),
        "fixture file should have been written by record mode"
    );

    // Verify the written fixture can be replayed
    let replayer = ironflow_core::providers::record_replay::RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &fixtures_dir,
    );
    let replayed = Agent::new()
        .prompt(prompt)
        .model(Model::HAIKU)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&replayer)
        .await
        .unwrap();

    assert_eq!(replayed.text(), "recorded answer");
    assert_eq!(replayed.cost_usd(), Some(0.002));
}

#[test]
fn hash_config_allowed_tools_affect_hash() {
    let mut c1 = ironflow_core::provider::AgentConfig::new("prompt");
    c1.allowed_tools = vec!["Read".to_string()];
    let mut c2 = ironflow_core::provider::AgentConfig::new("prompt");
    c2.allowed_tools = vec!["Write".to_string()];
    assert_ne!(hash_config(&c1), hash_config(&c2));
}

#[test]
fn hash_config_model_affects_hash() {
    let mut c1 = ironflow_core::provider::AgentConfig::new("prompt");
    c1.model = Model::SONNET.to_string();
    let mut c2 = ironflow_core::provider::AgentConfig::new("prompt");
    c2.model = Model::OPUS.to_string();
    assert_ne!(hash_config(&c1), hash_config(&c2));
}

#[test]
fn hash_config_resume_session_affects_hash() {
    let c1 = ironflow_core::provider::AgentConfig::new("prompt");
    let mut c2 = ironflow_core::provider::AgentConfig::new("prompt");
    c2.resume_session_id = Some("sess-123".to_string());
    assert_ne!(hash_config(&c1), hash_config(&c2));
}

#[test]
fn hash_config_empty_prompt() {
    let c1 = ironflow_core::provider::AgentConfig::new("");
    let c2 = ironflow_core::provider::AgentConfig::new("");
    assert_eq!(hash_config(&c1), hash_config(&c2));
}
