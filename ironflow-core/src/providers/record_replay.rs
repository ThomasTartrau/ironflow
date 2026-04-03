//! Record/replay provider for deterministic testing.
//!
//! [`RecordReplayProvider`] wraps any [`AgentProvider`] and adds fixture-based
//! record/replay capability. This lets you capture real agent responses once and
//! replay them in tests without hitting the live API.
//!
//! # How it works
//!
//! 1. **Recording** - Set the `IRONFLOW_RECORD=1` environment variable. Each
//!    invocation is forwarded to the inner provider, and the request/response
//!    pair is saved as a JSON fixture file.
//! 2. **Replaying** - Without `IRONFLOW_RECORD`, the provider looks up a
//!    fixture by hash. If found, it returns the cached response instantly.
//!    If not found, it falls back to the inner provider with a warning.
//!
//! # Fixture naming
//!
//! Fixture files are named `{hash}.json` where the hash is derived from the
//! prompt, system prompt, and JSON schema. This means identical configurations
//! always map to the same file, making fixtures stable across runs.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let inner = ClaudeCodeProvider::new();
//! let provider = RecordReplayProvider::new(inner, "tests/fixtures");
//!
//! // With IRONFLOW_RECORD=1: calls Claude and saves the response.
//! // Without IRONFLOW_RECORD: replays from tests/fixtures/{hash}.json.
//! let result = Agent::new()
//!     .prompt("Explain ownership in Rust")
//!     .run(&provider)
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{from_str, to_string_pretty};
use tracing::{info, warn};

use crate::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};

#[derive(Serialize, Deserialize)]
struct Fixture {
    config: AgentConfig,
    output: AgentOutput,
}

/// Compute the fixture filename hash for a given config.
///
/// The hash is derived from the prompt, system prompt, and JSON schema fields.
/// Identical configurations always produce the same hash, making fixture
/// filenames stable across runs.
///
/// This is a standalone function so callers don't need to specify a provider
/// type parameter. Use it to pre-compute hashes when creating fixtures manually
/// in tests.
pub fn hash_config(config: &AgentConfig) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(config.prompt.as_bytes());
    if let Some(ref sp) = config.system_prompt {
        hasher.update(b"|sp:");
        hasher.update(sp.as_bytes());
    }
    hasher.update(b"|m:");
    hasher.update(config.model.to_string().as_bytes());
    if !config.allowed_tools.is_empty() {
        hasher.update(b"|at:");
        hasher.update(config.allowed_tools.join(",").as_bytes());
    }
    if let Some(ref schema) = config.json_schema {
        hasher.update(b"|js:");
        hasher.update(schema.as_bytes());
    }
    if let Some(ref session_id) = config.resume_session_id {
        hasher.update(b"|rs:");
        hasher.update(session_id.as_bytes());
    }
    let result = hasher.finalize();
    format!(
        "{:016x}",
        u64::from_be_bytes(result[..8].try_into().unwrap())
    )
}

/// A test-oriented [`AgentProvider`] wrapper that records and replays agent
/// responses from JSON fixture files.
///
/// See the [module-level documentation](self) for usage details.
pub struct RecordReplayProvider<P: AgentProvider> {
    inner: P,
    fixtures_dir: PathBuf,
    recording: bool,
}

impl<P: AgentProvider> RecordReplayProvider<P> {
    /// Create a new record/replay provider wrapping `inner`.
    ///
    /// * `inner` - the real provider to delegate to when recording or when a
    ///   fixture is missing during replay.
    /// * `fixtures_dir` - directory where fixture JSON files are stored. Created
    ///   automatically when recording is enabled.
    ///
    /// Recording mode is activated when the `IRONFLOW_RECORD` environment
    /// variable is set (to any value).
    pub fn new(inner: P, fixtures_dir: &str) -> Self {
        let recording = std::env::var("IRONFLOW_RECORD").is_ok();
        if recording && let Err(e) = fs::create_dir_all(fixtures_dir) {
            warn!(path = %fixtures_dir, error = %e, "failed to create fixtures directory - recordings will fail");
        }
        Self {
            inner,
            fixtures_dir: PathBuf::from(fixtures_dir),
            recording,
        }
    }

    /// Create a provider that **always replays** from fixtures, ignoring the
    /// `IRONFLOW_RECORD` environment variable.
    ///
    /// If a fixture is not found, falls through to the inner provider.
    /// Use this in tests to guarantee deterministic behavior regardless of
    /// the environment.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::prelude::*;
    /// use ironflow_core::providers::record_replay::RecordReplayProvider;
    ///
    /// let provider = RecordReplayProvider::replay(
    ///     ClaudeCodeProvider::new(),
    ///     "tests/fixtures",
    /// );
    /// ```
    pub fn replay(inner: P, fixtures_dir: &str) -> Self {
        Self {
            inner,
            fixtures_dir: PathBuf::from(fixtures_dir),
            recording: false,
        }
    }

    /// Create a provider that **always records**, ignoring the
    /// `IRONFLOW_RECORD` environment variable.
    ///
    /// Every invocation is forwarded to the inner provider and the response
    /// is saved as a fixture.
    pub fn record(inner: P, fixtures_dir: &str) -> Self {
        fs::create_dir_all(fixtures_dir).ok();
        Self {
            inner,
            fixtures_dir: PathBuf::from(fixtures_dir),
            recording: true,
        }
    }

    fn fixture_path(&self, config: &AgentConfig) -> PathBuf {
        let hash = hash_config(config);
        self.fixtures_dir.join(format!("{hash}.json"))
    }

    fn load_fixture(&self, path: &Path) -> Option<AgentOutput> {
        let content = fs::read_to_string(path).ok()?;
        let fixture: Fixture = from_str(&content).ok()?;
        Some(fixture.output)
    }

    fn save_fixture(&self, path: &Path, config: &AgentConfig, output: &AgentOutput) {
        let fixture = Fixture {
            config: config.clone(),
            output: output.clone(),
        };
        if let Ok(json) = to_string_pretty(&fixture)
            && let Err(e) = fs::write(path, json)
        {
            warn!(path = %path.display(), error = %e, "failed to save fixture");
        }
    }
}

impl<P: AgentProvider> AgentProvider for RecordReplayProvider<P> {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            let path = self.fixture_path(config);

            if !self.recording {
                if let Some(output) = self.load_fixture(&path) {
                    info!(fixture = %path.display(), "replaying from fixture");
                    return Ok(output);
                }
                warn!(fixture = %path.display(), "fixture not found, calling real provider");
            }

            let output = self.inner.invoke(config).await?;

            if self.recording {
                self.save_fixture(&path, config, &output);
                info!(fixture = %path.display(), "recorded fixture");
            }

            Ok(output)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::*;
    use crate::operations::agent::Model;
    use crate::providers::claude::ClaudeCodeProvider;

    /// RAII guard that removes a temp directory on drop (even on panic).
    struct TempDirGuard(String);

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn temp_fixtures_dir() -> (String, TempDirGuard) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = format!(
            "/tmp/ironflow-test-rr-{}-{}",
            process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        let guard = TempDirGuard(dir.clone());
        (dir, guard)
    }

    /// Helper: create a provider wrapping a real ClaudeCodeProvider in replay-only mode.
    /// Since no fixtures exist by default, it will fall back to the inner provider
    /// only when explicitly needed. For most tests we write fixtures manually.
    fn replay_provider(fixtures_dir: &str) -> RecordReplayProvider<ClaudeCodeProvider> {
        RecordReplayProvider::replay(ClaudeCodeProvider::new(), fixtures_dir)
    }

    fn record_provider(fixtures_dir: &str) -> RecordReplayProvider<ClaudeCodeProvider> {
        RecordReplayProvider::record(ClaudeCodeProvider::new(), fixtures_dir)
    }

    /// Write a fixture file to disk for a given config.
    fn write_fixture(fixtures_dir: &str, config: &AgentConfig, output: &AgentOutput) {
        fs::create_dir_all(fixtures_dir).unwrap();
        let hash = hash_config(config);
        let fixture = Fixture {
            config: config.clone(),
            output: output.clone(),
        };
        let json = serde_json::to_string_pretty(&fixture).unwrap();
        fs::write(format!("{}/{}.json", fixtures_dir, hash), json).unwrap();
    }

    fn sample_output() -> AgentOutput {
        AgentOutput {
            value: json!("Rust is a systems programming language"),
            session_id: None,
            cost_usd: Some(0.01),
            input_tokens: Some(10),
            output_tokens: Some(50),
            model: Some("claude-sonnet".to_string()),
            duration_ms: 100,
        }
    }

    // ──── hash_config tests ────────────────────────────────────────

    #[test]
    fn test_hash_config_deterministic() {
        let config = AgentConfig::new("What is Rust?");
        let hash1 = hash_config(&config);
        let hash2 = hash_config(&config);
        assert_eq!(hash1, hash2, "same config should produce same hash");
    }

    #[test]
    fn test_hash_config_different_prompts() {
        let config1 = AgentConfig::new("Prompt A");
        let config2 = AgentConfig::new("Prompt B");
        assert_ne!(hash_config(&config1), hash_config(&config2));
    }

    #[test]
    fn test_hash_config_system_prompt_affects_hash() {
        let mut config1 = AgentConfig::new("Analyze this code");
        let mut config2 = AgentConfig::new("Analyze this code");
        config1.system_prompt = Some("You are a Rust expert".to_string());
        config2.system_prompt = Some("You are a Python expert".to_string());
        assert_ne!(hash_config(&config1), hash_config(&config2));
    }

    #[test]
    fn test_hash_config_no_system_prompt_vs_with_system_prompt() {
        let config1 = AgentConfig::new("Analyze this code");
        let mut config2 = AgentConfig::new("Analyze this code");
        config2.system_prompt = Some("You are a Rust expert".to_string());
        assert_ne!(hash_config(&config1), hash_config(&config2));
    }

    #[test]
    fn test_hash_config_model_affects_hash() {
        let mut config1 = AgentConfig::new("What is Rust?");
        let mut config2 = AgentConfig::new("What is Rust?");
        config1.model = Model::SONNET.to_string();
        config2.model = Model::OPUS.to_string();
        assert_ne!(hash_config(&config1), hash_config(&config2));
    }

    #[test]
    fn test_hash_config_allowed_tools_affects_hash() {
        let mut config1 = AgentConfig::new("Write code");
        let mut config2 = AgentConfig::new("Write code");
        config1.allowed_tools = vec!["Read".to_string(), "Write".to_string()];
        config2.allowed_tools = vec!["Read".to_string()];
        assert_ne!(hash_config(&config1), hash_config(&config2));
    }

    #[test]
    fn test_hash_config_json_schema_affects_hash() {
        let mut config1 = AgentConfig::new("Extract data");
        let mut config2 = AgentConfig::new("Extract data");
        config1.json_schema = Some(r#"{"type": "object"}"#.to_string());
        config2.json_schema = Some(r#"{"type": "array"}"#.to_string());
        assert_ne!(hash_config(&config1), hash_config(&config2));
    }

    #[test]
    fn test_hash_config_resume_session_id_affects_hash() {
        let mut config1 = AgentConfig::new("Continue conversation");
        let mut config2 = AgentConfig::new("Continue conversation");
        config1.resume_session_id = Some("session-123".to_string());
        config2.resume_session_id = Some("session-456".to_string());
        assert_ne!(hash_config(&config1), hash_config(&config2));
    }

    #[test]
    fn test_hash_config_format() {
        let config = AgentConfig::new("Test");
        let hash = hash_config(&config);
        assert_eq!(hash.len(), 16, "hash should be 16 hex characters");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_multiple_configs_produce_unique_hashes() {
        let configs = [
            AgentConfig::new("Prompt 1"),
            AgentConfig::new("Prompt 2"),
            AgentConfig::new("Prompt 3"),
        ];
        let hashes: Vec<String> = configs.iter().map(hash_config).collect();
        for i in 0..hashes.len() {
            for j in (i + 1)..hashes.len() {
                assert_ne!(hashes[i], hashes[j], "hashes should be unique");
            }
        }
    }

    // ──── replay: reads fixture from disk ─────────────────────────

    #[tokio::test]
    async fn test_replay_returns_fixture_when_present() {
        let (dir, _guard) = temp_fixtures_dir();
        let config = AgentConfig::new("What is Rust?");
        let expected = sample_output();
        write_fixture(&dir, &config, &expected);

        let provider = replay_provider(&dir);
        let result = provider.invoke(&config).await.unwrap();

        assert_eq!(result.value, expected.value);
        assert_eq!(result.cost_usd, expected.cost_usd);
        assert_eq!(result.input_tokens, expected.input_tokens);
    }

    // ──── fixture_path naming ─────────────────────────────────────

    #[test]
    fn test_fixture_path_uses_hash_config() {
        let (dir, _guard) = temp_fixtures_dir();
        let config = AgentConfig::new("Test prompt");
        let provider = replay_provider(&dir);

        let path = provider.fixture_path(&config);
        let expected = format!("{}/{}.json", dir, hash_config(&config));
        assert_eq!(path.to_string_lossy().to_string(), expected);
    }

    #[test]
    fn test_fixture_path_different_configs_different_files() {
        let (dir, _guard) = temp_fixtures_dir();
        let provider = replay_provider(&dir);

        let path1 = provider.fixture_path(&AgentConfig::new("Prompt A"));
        let path2 = provider.fixture_path(&AgentConfig::new("Prompt B"));
        assert_ne!(path1, path2);
    }

    // ──── load_fixture ────────────────────────────────────────────

    #[test]
    fn test_load_fixture_returns_none_when_file_missing() {
        let (dir, _guard) = temp_fixtures_dir();
        let provider = replay_provider(&dir);
        let result = provider.load_fixture(Path::new("/tmp/nonexistent-xyz.json"));
        assert!(result.is_none());
    }

    #[test]
    fn test_load_fixture_returns_none_when_file_malformed() {
        let (dir, _guard) = temp_fixtures_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = format!("{}/malformed.json", dir);
        fs::write(&path, "{ invalid json }").unwrap();

        let provider = replay_provider(&dir);
        assert!(provider.load_fixture(Path::new(&path)).is_none());
    }

    #[test]
    fn test_load_fixture_extracts_output_from_valid_fixture() {
        let (dir, _guard) = temp_fixtures_dir();
        let config = AgentConfig::new("Test");
        let expected = sample_output();
        write_fixture(&dir, &config, &expected);

        let provider = replay_provider(&dir);
        let hash = hash_config(&config);
        let path = format!("{}/{}.json", dir, hash);

        let loaded = provider.load_fixture(Path::new(&path)).unwrap();
        assert_eq!(loaded.value, expected.value);
        assert_eq!(loaded.cost_usd, expected.cost_usd);
    }

    // ──── save_fixture ────────────────────────────────────────────

    #[test]
    fn test_save_fixture_creates_valid_json_file() {
        let (dir, _guard) = temp_fixtures_dir();
        fs::create_dir_all(&dir).unwrap();

        let config = AgentConfig::new("Save test");
        let output = AgentOutput::new(json!({"key": "value", "nested": {"count": 42}}));

        let provider = replay_provider(&dir);
        let path = provider.fixture_path(&config);
        provider.save_fixture(&path, &config, &output);

        assert!(path.exists());

        let saved_json = fs::read_to_string(&path).unwrap();
        let fixture: Fixture = serde_json::from_str(&saved_json).unwrap();
        assert_eq!(fixture.config.prompt, config.prompt);
        assert_eq!(fixture.output.value, output.value);
    }

    // ──── save then load roundtrip ────────────────────────────────

    #[test]
    fn test_save_then_load_roundtrip() {
        let (dir, _guard) = temp_fixtures_dir();
        fs::create_dir_all(&dir).unwrap();

        let config = AgentConfig::new("Roundtrip");
        let output = AgentOutput {
            value: json!("ownership is key"),
            session_id: Some("sess-42".to_string()),
            cost_usd: Some(0.05),
            input_tokens: Some(20),
            output_tokens: Some(100),
            model: Some("claude-sonnet".to_string()),
            duration_ms: 500,
        };

        let provider = replay_provider(&dir);
        let path = provider.fixture_path(&config);
        provider.save_fixture(&path, &config, &output);

        let loaded = provider.load_fixture(&path).unwrap();
        assert_eq!(loaded.value, output.value);
        assert_eq!(loaded.session_id, output.session_id);
        assert_eq!(loaded.cost_usd, output.cost_usd);
        assert_eq!(loaded.input_tokens, output.input_tokens);
        assert_eq!(loaded.output_tokens, output.output_tokens);
    }

    // ──── Edge cases ──────────────────────────────────────────────

    #[test]
    fn test_empty_prompt_hash() {
        let config = AgentConfig::new("");
        let hash = hash_config(&config);
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn test_unicode_prompt_roundtrip() {
        let (dir, _guard) = temp_fixtures_dir();
        fs::create_dir_all(&dir).unwrap();

        let config = AgentConfig::new("Explain: 你好世界 🌍 Здравствуй мир");
        let output = AgentOutput::new(json!("Unicode response"));

        let provider = replay_provider(&dir);
        let path = provider.fixture_path(&config);
        provider.save_fixture(&path, &config, &output);

        let loaded = provider.load_fixture(&path).unwrap();
        assert_eq!(loaded.value, output.value);
    }

    #[test]
    fn test_new_defaults_to_replay_mode() {
        let provider = RecordReplayProvider::new(ClaudeCodeProvider::new(), "/tmp/doesnt-matter");
        assert!(!provider.recording);
    }

    #[test]
    fn test_record_mode_flag() {
        let (dir, _guard) = temp_fixtures_dir();
        let provider = record_provider(&dir);
        assert!(provider.recording);
    }

    #[test]
    fn test_replay_mode_flag() {
        let (dir, _guard) = temp_fixtures_dir();
        let provider = replay_provider(&dir);
        assert!(!provider.recording);
    }
}
