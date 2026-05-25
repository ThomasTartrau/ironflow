//! Anthropic Messages API provider (direct HTTP, not CLI).

pub mod adapter;

pub use adapter::AnthropicModel;

use std::env;

use crate::providers::http::adapter::HttpAgentProvider;

use self::adapter::AnthropicApiAdapter;

/// Anthropic Messages API provider.
///
/// Uses the HTTP API directly (not the Claude CLI). Supports structured output
/// via tool forcing.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
/// use ironflow_core::providers::http::AnthropicApiProvider;
///
/// # async fn example() -> Result<(), OperationError> {
/// let provider = AnthropicApiProvider::from_env();
/// let result = Agent::new()
///     .prompt("Say hello")
///     .model("claude-sonnet-4-20250514")
///     .run(&provider)
///     .await?;
/// # Ok(())
/// # }
/// ```
pub type AnthropicApiProvider = HttpAgentProvider<AnthropicApiAdapter>;

impl AnthropicApiProvider {
    /// Create from the `ANTHROPIC_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `ANTHROPIC_API_KEY` is not set.
    pub fn from_env() -> Self {
        let api_key = env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set");
        HttpAgentProvider::new(AnthropicApiAdapter::new(api_key))
    }

    /// Create with an explicit API key.
    pub fn with_api_key(api_key: String) -> Self {
        HttpAgentProvider::new(AnthropicApiAdapter::new(api_key))
    }
}
