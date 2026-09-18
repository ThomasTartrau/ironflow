//! [`RecordReplayDecisionProvider`] -- fixture-based record/replay for
//! [`DecisionProvider`], the analogue of
//! [`RecordReplayProvider`](crate::providers::record_replay::RecordReplayProvider).
//!
//! System One backends (Jev) have no local process to shell out to, so the common
//! test mode is **replay only**: construct with [`replay`](RecordReplayDecisionProvider::replay)
//! and no backend at all, and every [`decide`](crate::decision::DecisionProvider::decide)
//! is served from a captured JSON fixture. Recording (against a real provider) is
//! also supported via [`with_inner`](RecordReplayDecisionProvider::with_inner) and
//! [`record`](RecordReplayDecisionProvider::record).
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::providers::record_replay_decision::RecordReplayDecisionProvider;
//!
//! // Replay-only: no backend required.
//! let provider = RecordReplayDecisionProvider::replay("tests/fixtures/decisions");
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{from_str, to_string_pretty};
use tracing::{info, warn};

use crate::decision::{DecideFuture, DecisionOutput, DecisionProvider, DecisionRequest};
use crate::error::AgentError;

const PROVIDER_NAME: &str = "record-replay-decision";

#[derive(Serialize, Deserialize)]
struct Fixture {
    request: DecisionRequest,
    output: DecisionOutput,
}

/// Compute the fixture filename hash for a decision request.
///
/// The hash is derived from the canonical JSON of the request (state, model, and
/// the full question map). Identical requests always produce the same hash, so
/// fixture filenames are stable across runs.
///
/// # Examples
///
/// ```
/// use ironflow_core::decision::DecisionRequest;
/// use ironflow_core::providers::record_replay_decision::hash_request;
/// use std::collections::BTreeMap;
/// use serde_json::json;
///
/// let request = DecisionRequest {
///     state: json!("hello"),
///     model: "jev-latest".into(),
///     questions: BTreeMap::new(),
/// };
/// let h1 = hash_request(&request);
/// let h2 = hash_request(&request);
/// assert_eq!(h1, h2);
/// ```
pub fn hash_request(request: &DecisionRequest) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_string(request).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let result = hasher.finalize();
    format!(
        "{:016x}",
        u64::from_be_bytes(result[..8].try_into().unwrap())
    )
}

/// A test-oriented [`DecisionProvider`] that records and replays decision
/// responses from JSON fixture files.
///
/// See the [module-level documentation](self) for usage.
pub struct RecordReplayDecisionProvider {
    inner: Option<Arc<dyn DecisionProvider>>,
    fixtures_dir: PathBuf,
    recording: bool,
}

impl RecordReplayDecisionProvider {
    /// A **replay-only** provider: every request is served from a fixture, and
    /// there is no backend to fall through to. Ignores `IRONFLOW_RECORD`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::record_replay_decision::RecordReplayDecisionProvider;
    ///
    /// let provider = RecordReplayDecisionProvider::replay("tests/fixtures/decisions");
    /// ```
    pub fn replay(fixtures_dir: &str) -> Self {
        Self {
            inner: None,
            fixtures_dir: PathBuf::from(fixtures_dir),
            recording: false,
        }
    }

    /// Wrap a real provider, recording when `IRONFLOW_RECORD` is set and
    /// replaying otherwise (falling through to `inner` on a missing fixture).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::decision::{DecideFuture, DecisionOutput, DecisionProvider, DecisionRequest, DecisionUsage};
    /// use ironflow_core::providers::record_replay_decision::RecordReplayDecisionProvider;
    /// use std::collections::BTreeMap;
    /// use std::sync::Arc;
    ///
    /// struct MyBackend;
    /// impl DecisionProvider for MyBackend {
    ///     fn decide<'a>(&'a self, _request: &'a DecisionRequest) -> DecideFuture<'a> {
    ///         Box::pin(async {
    ///             Ok(DecisionOutput { model: None, answers: BTreeMap::new(), usage: DecisionUsage::default() })
    ///         })
    ///     }
    /// }
    ///
    /// let provider = RecordReplayDecisionProvider::with_inner(
    ///     Arc::new(MyBackend),
    ///     "tests/fixtures/decisions",
    /// );
    /// ```
    pub fn with_inner(inner: Arc<dyn DecisionProvider>, fixtures_dir: &str) -> Self {
        let recording = std::env::var("IRONFLOW_RECORD").is_ok();
        if recording && let Err(e) = fs::create_dir_all(fixtures_dir) {
            warn!(path = %fixtures_dir, error = %e, "failed to create fixtures directory");
        }
        Self {
            inner: Some(inner),
            fixtures_dir: PathBuf::from(fixtures_dir),
            recording,
        }
    }

    /// Wrap a real provider and **always record**, ignoring `IRONFLOW_RECORD`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::decision::{DecideFuture, DecisionOutput, DecisionProvider, DecisionRequest, DecisionUsage};
    /// use ironflow_core::providers::record_replay_decision::RecordReplayDecisionProvider;
    /// use std::collections::BTreeMap;
    /// use std::sync::Arc;
    ///
    /// struct MyBackend;
    /// impl DecisionProvider for MyBackend {
    ///     fn decide<'a>(&'a self, _request: &'a DecisionRequest) -> DecideFuture<'a> {
    ///         Box::pin(async {
    ///             Ok(DecisionOutput { model: None, answers: BTreeMap::new(), usage: DecisionUsage::default() })
    ///         })
    ///     }
    /// }
    ///
    /// let provider = RecordReplayDecisionProvider::record(
    ///     Arc::new(MyBackend),
    ///     "tests/fixtures/decisions",
    /// );
    /// ```
    pub fn record(inner: Arc<dyn DecisionProvider>, fixtures_dir: &str) -> Self {
        fs::create_dir_all(fixtures_dir).ok();
        Self {
            inner: Some(inner),
            fixtures_dir: PathBuf::from(fixtures_dir),
            recording: true,
        }
    }

    /// Directory where fixtures are stored.
    pub fn fixtures_dir(&self) -> &Path {
        &self.fixtures_dir
    }

    fn fixture_path(&self, request: &DecisionRequest) -> PathBuf {
        self.fixtures_dir
            .join(format!("{}.json", hash_request(request)))
    }

    fn load_fixture(&self, path: &Path) -> Option<DecisionOutput> {
        let content = fs::read_to_string(path).ok()?;
        let fixture: Fixture = from_str(&content).ok()?;
        Some(fixture.output)
    }

    fn save_fixture(&self, path: &Path, request: &DecisionRequest, output: &DecisionOutput) {
        let fixture = Fixture {
            request: request.clone(),
            output: output.clone(),
        };
        if let Ok(json) = to_string_pretty(&fixture)
            && let Err(e) = fs::write(path, json)
        {
            warn!(path = %path.display(), error = %e, "failed to save decision fixture");
        }
    }
}

impl DecisionProvider for RecordReplayDecisionProvider {
    fn decide<'a>(&'a self, request: &'a DecisionRequest) -> DecideFuture<'a> {
        Box::pin(async move {
            let path = self.fixture_path(request);

            if !self.recording
                && let Some(output) = self.load_fixture(&path)
            {
                info!(fixture = %path.display(), "replaying decision from fixture");
                return Ok(output);
            }

            let Some(inner) = self.inner.as_ref() else {
                return Err(AgentError::HttpProvider {
                    provider: PROVIDER_NAME.to_string(),
                    status_code: 0,
                    message: format!(
                        "no fixture at {} and no inner decision provider to fall through to",
                        path.display()
                    ),
                });
            };

            let output = inner.decide(request).await?;
            if self.recording {
                self.save_fixture(&path, request, &output);
                info!(fixture = %path.display(), "recorded decision fixture");
            }
            Ok(output)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::*;
    use crate::decision::{DecisionAnswer, DecisionUsage, NoulAnswer};

    struct TempDirGuard(String);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn temp_dir() -> (String, TempDirGuard) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = format!(
            "/tmp/ironflow-test-rrd-{}-{}",
            process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        fs::create_dir_all(&dir).unwrap();
        let guard = TempDirGuard(dir.clone());
        (dir, guard)
    }

    fn sample_request() -> DecisionRequest {
        DecisionRequest {
            state: json!("payouts failing"),
            model: "jev-latest".into(),
            questions: BTreeMap::new(),
        }
    }

    fn write_fixture(dir: &str, request: &DecisionRequest, output: &DecisionOutput) {
        let fixture = Fixture {
            request: request.clone(),
            output: output.clone(),
        };
        let path = PathBuf::from(dir).join(format!("{}.json", hash_request(request)));
        fs::write(path, to_string_pretty(&fixture).unwrap()).unwrap();
    }

    #[tokio::test]
    async fn replay_returns_fixture() {
        let (dir, _guard) = temp_dir();
        let request = sample_request();
        let output = DecisionOutput {
            model: Some("jev-latest".into()),
            answers: BTreeMap::from([(
                "q".to_string(),
                DecisionAnswer::Noul(NoulAnswer { noul: 0.9 }),
            )]),
            usage: DecisionUsage {
                input_tokens: 100,
                output_tokens: 0,
            },
        };
        write_fixture(&dir, &request, &output);

        let provider = RecordReplayDecisionProvider::replay(&dir);
        let got = provider.decide(&request).await.unwrap();
        assert_eq!(got.noul("q").unwrap(), 0.9);
        assert_eq!(got.usage.input_tokens, 100);
    }

    #[tokio::test]
    async fn replay_without_fixture_and_without_inner_errors() {
        let (dir, _guard) = temp_dir();
        let provider = RecordReplayDecisionProvider::replay(&dir);
        let err = provider.decide(&sample_request()).await.unwrap_err();
        assert!(matches!(
            err,
            AgentError::HttpProvider { status_code: 0, .. }
        ));
    }

    #[test]
    fn hash_is_stable_and_distinct() {
        let a = sample_request();
        let mut b = sample_request();
        b.state = json!("something else");
        assert_eq!(hash_request(&a), hash_request(&a));
        assert_ne!(hash_request(&a), hash_request(&b));
    }
}
