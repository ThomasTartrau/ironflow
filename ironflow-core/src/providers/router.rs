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

use std::sync::Arc;

use tracing::debug;

use crate::provider::{AgentConfig, AgentProvider, InvokeFuture, LogSink};

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
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;
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
