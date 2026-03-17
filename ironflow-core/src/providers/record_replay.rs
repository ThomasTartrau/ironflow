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
