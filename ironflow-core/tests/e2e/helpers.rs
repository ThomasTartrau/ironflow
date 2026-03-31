use std::fs;
use std::path::PathBuf;

use ironflow_core::prelude::*;
use ironflow_core::provider::{AgentConfig, AgentOutput};
use ironflow_core::providers::record_replay::hash_config;
use serde_json::json;

/// Write a fixture JSON file that RecordReplayProvider will find.
pub fn write_fixture(fixtures_dir: &str, config: &AgentConfig, output: &AgentOutput) {
    fs::create_dir_all(fixtures_dir).unwrap();
    let hash = hash_config(config);
    let path = PathBuf::from(fixtures_dir).join(format!("{hash}.json"));
    let fixture = json!({
        "config": config,
        "output": output,
    });
    fs::write(path, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
}

/// Build an AgentConfig matching what Agent builder produces.
pub fn make_config(
    prompt: &str,
    system_prompt: Option<&str>,
    json_schema: Option<&str>,
) -> AgentConfig {
    let mut config = AgentConfig::new(prompt);
    config.system_prompt = system_prompt.map(|s| s.to_string());
    config.model = Model::HAIKU.to_string();
    config.max_turns = Some(1);
    config.max_budget_usd = Some(0.10);
    config.json_schema = json_schema.map(|s| s.to_string());
    config
}

/// Build a test AgentOutput with known values.
pub fn make_output(
    value: serde_json::Value,
    cost: f64,
    input: u64,
    output_tok: u64,
) -> AgentOutput {
    let mut output = AgentOutput::new(value);
    output.session_id = Some("test-session-001".to_string());
    output.cost_usd = Some(cost);
    output.input_tokens = Some(input);
    output.output_tokens = Some(output_tok);
    output.model = Some("claude-haiku-4-5".to_string());
    output.duration_ms = 500;
    output
}

/// RAII guard that removes the fixtures directory on drop.
pub struct FixtureGuard {
    dir: String,
}

impl FixtureGuard {
    pub fn new(dir: &str) -> Self {
        Self {
            dir: dir.to_string(),
        }
    }
}

impl Drop for FixtureGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Create a unique temporary fixtures directory with automatic cleanup.
pub fn temp_fixtures_dir(name: &str) -> (String, FixtureGuard) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let dir = format!(
        "/tmp/ironflow-test-fixtures-{}-{}-{}",
        name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
    );
    let guard = FixtureGuard { dir: dir.clone() };
    (dir, guard)
}
