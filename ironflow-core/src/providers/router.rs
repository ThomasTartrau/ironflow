//! Provider router for multi-provider workflows.
//!
//! [`ProviderRouter`] implements [`AgentProvider`] and dispatches invocations
//! to different providers based on the model name or other criteria.
//!
//! # Examples
//!
//! ```no_run
//! use std::sync::Arc;
//! use ironflow_core::providers::router::{ProviderRouter, ProviderMatcher};
//! use ironflow_core::providers::claude::ClaudeCodeProvider;
//! use ironflow_core::provider::AgentProvider;
//!
//! let claude = Arc::new(ClaudeCodeProvider::new());
//! // let nvidia = Arc::new(nvidia_provider);
//!
//! let router = ProviderRouter::new(claude.clone())
//!     // .route(ProviderMatcher::ModelPrefix("nvidia/".into()), nvidia)
//!     ;
//!
//! // router implements AgentProvider, pass it to Engine as usual
//! let provider: Arc<dyn AgentProvider> = Arc::new(router);
//! ```

use std::iter::once;
use std::sync::Arc;

use tracing::debug;

use crate::provider::{AgentConfig, AgentProvider, InvokeFuture, LogSink, ReleaseFuture};

/// Matching strategy for routing invocations to providers.
#[derive(Debug, Clone)]
pub enum ProviderMatcher {
    /// Match when the model starts with the given prefix.
    ///
    /// Example: `ModelPrefix("nvidia/".into())` matches `"nvidia/deepseek-v4-flash"`.
    ModelPrefix(String),
    /// Match when the model is exactly the given string.
    ///
    /// Example: `ModelExact("sonnet".into())` matches only `"sonnet"`.
    ModelExact(String),
}

impl ProviderMatcher {
    fn matches(&self, config: &AgentConfig) -> bool {
        match self {
            Self::ModelPrefix(prefix) => config.model.starts_with(prefix.as_str()),
            Self::ModelExact(exact) => config.model == *exact,
        }
    }
}

/// Routes agent invocations to different providers based on model/config matching.
///
/// Evaluates matchers in registration order; first match wins. If no matcher
/// matches, the fallback provider handles the request.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_core::providers::router::{ProviderRouter, ProviderMatcher};
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use ironflow_core::provider::{AgentConfig, AgentProvider};
///
/// # async fn example() -> Result<(), ironflow_core::error::AgentError> {
/// let claude = Arc::new(ClaudeCodeProvider::new());
/// let router = ProviderRouter::new(claude.clone());
///
/// // Uses the fallback (claude) since no routes match "sonnet"
/// let config = AgentConfig::new("hello");
/// let output = router.invoke(&config).await?;
/// # Ok(())
/// # }
/// ```
pub struct ProviderRouter {
    routes: Vec<(ProviderMatcher, Arc<dyn AgentProvider>)>,
    fallback: Arc<dyn AgentProvider>,
}

impl ProviderRouter {
    /// Create a router with a fallback provider for unmatched models.
    pub fn new(fallback: Arc<dyn AgentProvider>) -> Self {
        Self {
            routes: Vec::new(),
            fallback,
        }
    }

    /// Add a routing rule. Routes are evaluated in order; first match wins.
    pub fn route(mut self, matcher: ProviderMatcher, provider: Arc<dyn AgentProvider>) -> Self {
        self.routes.push((matcher, provider));
        self
    }

    /// Resolve which provider handles a given config.
    fn resolve(&self, config: &AgentConfig) -> &Arc<dyn AgentProvider> {
        for (matcher, provider) in &self.routes {
            if matcher.matches(config) {
                debug!(
                    model = %config.model,
                    matcher = ?matcher,
                    "routed to matched provider"
                );
                return provider;
            }
        }
        debug!(model = %config.model, "using fallback provider");
        &self.fallback
    }
}

impl AgentProvider for ProviderRouter {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        let provider = self.resolve(config);
        provider.invoke(config)
    }

    fn invoke_with_logs<'a>(
        &'a self,
        config: &'a AgentConfig,
        log_sink: Arc<dyn LogSink>,
    ) -> InvokeFuture<'a> {
        let provider = self.resolve(config);
        provider.invoke_with_logs(config, log_sink)
    }

    /// The kind shared by the fallback and every routed provider, or `None`
    /// when they differ or any of them has no kind.
    ///
    /// Use [`AgentProvider::account_kind_for`] for the kind of a given config.
    fn account_kind(&self) -> Option<&'static str> {
        let first = self.fallback.account_kind()?;
        self.routes
            .iter()
            .all(|(_, provider)| provider.account_kind() == Some(first))
            .then_some(first)
    }

    /// The kind of the provider this config is routed to.
    fn account_kind_for(&self, config: &AgentConfig) -> Option<&'static str> {
        self.resolve(config).account_kind_for(config)
    }

    /// Release the run on the fallback and on every routed provider: any of
    /// them may have started something for it. A failure does not stop the
    /// others; the first one is returned once all have been asked.
    fn release_run<'a>(&'a self, run_id: &'a str) -> ReleaseFuture<'a> {
        Box::pin(async move {
            let routed = self.routes.iter().map(|(_, provider)| provider);
            let mut first_err = None;
            for provider in once(&self.fallback).chain(routed) {
                if let Err(e) = provider.release_run(run_id).await {
                    first_err.get_or_insert(e);
                }
            }
            first_err.map_or(Ok(()), Err)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;
    use crate::error::AgentError;
    use crate::provider::AgentOutput;

    struct CountingProvider {
        name: &'static str,
        count: AtomicUsize,
    }

    impl CountingProvider {
        fn new(name: &'static str) -> Arc<Self> {
            Arc::new(Self {
                name,
                count: AtomicUsize::new(0),
            })
        }

        fn call_count(&self) -> usize {
            self.count.load(Ordering::Relaxed)
        }
    }

    impl AgentProvider for CountingProvider {
        fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
            self.count.fetch_add(1, Ordering::Relaxed);
            let name = self.name;
            Box::pin(async move { Ok(AgentOutput::new(json!(name))) })
        }
    }

    struct KindProvider {
        kind: Option<&'static str>,
    }

    impl AgentProvider for KindProvider {
        fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
            Box::pin(async { Ok(AgentOutput::new(json!("kind"))) })
        }

        fn account_kind(&self) -> Option<&'static str> {
            self.kind
        }
    }

    fn kind_provider(kind: Option<&'static str>) -> Arc<KindProvider> {
        Arc::new(KindProvider { kind })
    }

    #[test]
    fn router_exposes_the_common_account_kind() {
        let router = ProviderRouter::new(kind_provider(Some("claude_subscription"))).route(
            ProviderMatcher::ModelPrefix("opus".into()),
            kind_provider(Some("claude_subscription")),
        );
        assert_eq!(router.account_kind(), Some("claude_subscription"));
    }

    #[test]
    fn router_without_routes_exposes_fallback_kind() {
        let router = ProviderRouter::new(kind_provider(Some("kind_a")));
        assert_eq!(router.account_kind(), Some("kind_a"));
        let router = ProviderRouter::new(kind_provider(None));
        assert_eq!(router.account_kind(), None);
    }

    #[test]
    fn router_with_mixed_kinds_exposes_no_common_kind() {
        let router = ProviderRouter::new(kind_provider(Some("claude_subscription"))).route(
            ProviderMatcher::ModelPrefix("gpt-".into()),
            kind_provider(None),
        );
        assert_eq!(router.account_kind(), None);

        let router = ProviderRouter::new(kind_provider(Some("kind_a"))).route(
            ProviderMatcher::ModelPrefix("gpt-".into()),
            kind_provider(Some("kind_b")),
        );
        assert_eq!(router.account_kind(), None);
    }

    #[test]
    fn router_account_kind_for_follows_the_routed_provider() {
        let router = ProviderRouter::new(kind_provider(Some("kind_a"))).route(
            ProviderMatcher::ModelPrefix("gpt-".into()),
            kind_provider(Some("kind_b")),
        );
        assert_eq!(
            router.account_kind_for(&AgentConfig::new("x").model("gpt-5")),
            Some("kind_b")
        );
        assert_eq!(
            router.account_kind_for(&AgentConfig::new("x").model("sonnet")),
            Some("kind_a")
        );

        let router = ProviderRouter::new(kind_provider(Some("kind_a"))).route(
            ProviderMatcher::ModelPrefix("gpt-".into()),
            kind_provider(None),
        );
        assert_eq!(
            router.account_kind_for(&AgentConfig::new("x").model("gpt-5")),
            None
        );
        assert_eq!(
            router.account_kind_for(&AgentConfig::new("x").model("sonnet")),
            Some("kind_a")
        );
    }

    /// Provider journaling the runs it is asked to release.
    #[derive(Default)]
    struct ReleaseProbe {
        released: Mutex<Vec<String>>,
        fail: bool,
    }

    impl AgentProvider for ReleaseProbe {
        fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
            Box::pin(async { Ok(AgentOutput::new(json!("probe"))) })
        }

        fn release_run<'a>(&'a self, run_id: &'a str) -> ReleaseFuture<'a> {
            Box::pin(async move {
                self.released.lock().unwrap().push(run_id.to_string());
                if self.fail {
                    return Err(AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: "release refused".to_string(),
                    });
                }
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn router_releases_the_run_on_every_provider() {
        let fallback = Arc::new(ReleaseProbe::default());
        let routed = Arc::new(ReleaseProbe::default());
        let router = ProviderRouter::new(fallback.clone())
            .route(ProviderMatcher::ModelPrefix("gpt".into()), routed.clone());

        router.release_run("run-1").await.expect("released");

        assert_eq!(*fallback.released.lock().unwrap(), vec!["run-1"]);
        assert_eq!(*routed.released.lock().unwrap(), vec!["run-1"]);
    }

    #[tokio::test]
    async fn router_release_failure_is_reported_after_releasing_the_others() {
        let failing = Arc::new(ReleaseProbe {
            fail: true,
            ..ReleaseProbe::default()
        });
        let routed = Arc::new(ReleaseProbe::default());
        let router = ProviderRouter::new(failing.clone())
            .route(ProviderMatcher::ModelPrefix("gpt".into()), routed.clone());

        let err = router.release_run("run-1").await.expect_err("fails");
        assert!(err.to_string().contains("release refused"), "{err}");
        assert_eq!(*failing.released.lock().unwrap(), vec!["run-1"]);
        assert_eq!(*routed.released.lock().unwrap(), vec!["run-1"]);
    }

    #[tokio::test]
    async fn router_fallback_when_no_routes() {
        let fallback = CountingProvider::new("fallback");
        let router = ProviderRouter::new(fallback.clone());

        let config = AgentConfig::new("hello");
        let output = router.invoke(&config).await.expect("should succeed");
        assert_eq!(output.value, json!("fallback"));
        assert_eq!(fallback.call_count(), 1);
    }

    #[tokio::test]
    async fn router_matches_model_prefix() {
        let fallback = CountingProvider::new("fallback");
        let nvidia = CountingProvider::new("nvidia");

        let router = ProviderRouter::new(fallback.clone()).route(
            ProviderMatcher::ModelPrefix("nvidia/".into()),
            nvidia.clone(),
        );

        let config = AgentConfig::new("hello").model("nvidia/deepseek-v4-flash");
        let output = router.invoke(&config).await.expect("should succeed");
        assert_eq!(output.value, json!("nvidia"));
        assert_eq!(nvidia.call_count(), 1);
        assert_eq!(fallback.call_count(), 0);
    }

    #[tokio::test]
    async fn router_matches_model_exact() {
        let fallback = CountingProvider::new("fallback");
        let special = CountingProvider::new("special");

        let router = ProviderRouter::new(fallback.clone()).route(
            ProviderMatcher::ModelExact("my-model".into()),
            special.clone(),
        );

        let config = AgentConfig::new("hello").model("my-model");
        let output = router.invoke(&config).await.expect("should succeed");
        assert_eq!(output.value, json!("special"));
        assert_eq!(special.call_count(), 1);
    }

    #[tokio::test]
    async fn router_exact_does_not_match_prefix() {
        let fallback = CountingProvider::new("fallback");
        let special = CountingProvider::new("special");

        let router = ProviderRouter::new(fallback.clone()).route(
            ProviderMatcher::ModelExact("nvidia".into()),
            special.clone(),
        );

        let config = AgentConfig::new("hello").model("nvidia/something");
        let output = router.invoke(&config).await.expect("should succeed");
        assert_eq!(output.value, json!("fallback"));
        assert_eq!(special.call_count(), 0);
        assert_eq!(fallback.call_count(), 1);
    }

    #[tokio::test]
    async fn router_first_match_wins() {
        let fallback = CountingProvider::new("fallback");
        let first = CountingProvider::new("first");
        let second = CountingProvider::new("second");

        let router = ProviderRouter::new(fallback.clone())
            .route(
                ProviderMatcher::ModelPrefix("nvidia/".into()),
                first.clone(),
            )
            .route(
                ProviderMatcher::ModelPrefix("nvidia/".into()),
                second.clone(),
            );

        let config = AgentConfig::new("hello").model("nvidia/test");
        let output = router.invoke(&config).await.expect("should succeed");
        assert_eq!(output.value, json!("first"));
        assert_eq!(first.call_count(), 1);
        assert_eq!(second.call_count(), 0);
    }

    #[tokio::test]
    async fn router_multiple_routes() {
        let fallback = CountingProvider::new("claude");
        let nvidia = CountingProvider::new("nvidia");
        let openai = CountingProvider::new("openai");

        let router = ProviderRouter::new(fallback.clone())
            .route(
                ProviderMatcher::ModelPrefix("nvidia/".into()),
                nvidia.clone(),
            )
            .route(ProviderMatcher::ModelPrefix("gpt-".into()), openai.clone());

        let config1 = AgentConfig::new("hello").model("nvidia/nemotron");
        let config2 = AgentConfig::new("hello").model("gpt-5.5");
        let config3 = AgentConfig::new("hello").model("sonnet");

        let out1 = router.invoke(&config1).await.expect("should succeed");
        let out2 = router.invoke(&config2).await.expect("should succeed");
        let out3 = router.invoke(&config3).await.expect("should succeed");

        assert_eq!(out1.value, json!("nvidia"));
        assert_eq!(out2.value, json!("openai"));
        assert_eq!(out3.value, json!("claude"));
    }
}
