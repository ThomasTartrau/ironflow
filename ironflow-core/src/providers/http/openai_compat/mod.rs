//! OpenAI-compatible provider implementation (covers OpenAI and Mistral).

pub mod adapter;
pub mod config;

pub use config::{MistralModel, OpenAiModel};

use crate::providers::http::adapter::HttpAgentProvider;

use self::adapter::OpenAiCompatAdapter;
use self::config::{MistralConfig, OpenAiConfig};

/// OpenAI provider using the Chat Completions API.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
/// use ironflow_core::providers::http::OpenAiProvider;
///
/// # async fn example() -> Result<(), OperationError> {
/// let provider = OpenAiProvider::from_env();
/// let result = Agent::new()
///     .prompt("Say hello")
///     .model("gpt-5.5")
///     .run(&provider)
///     .await?;
/// # Ok(())
/// # }
/// ```
pub type OpenAiProvider = HttpAgentProvider<OpenAiCompatAdapter<OpenAiConfig>>;

/// Mistral provider using the Chat Completions API.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
/// use ironflow_core::providers::http::MistralProvider;
///
/// # async fn example() -> Result<(), OperationError> {
/// let provider = MistralProvider::from_env();
/// let result = Agent::new()
///     .prompt("Say hello")
///     .model("mistral-medium-3505")
///     .run(&provider)
///     .await?;
/// # Ok(())
/// # }
/// ```
pub type MistralProvider = HttpAgentProvider<OpenAiCompatAdapter<MistralConfig>>;

impl OpenAiProvider {
    /// Create an OpenAI provider from the `OPENAI_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `OPENAI_API_KEY` is not set.
    pub fn from_env() -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(OpenAiConfig::from_env()))
    }

    /// Create an OpenAI provider with explicit credentials.
    pub fn with_credentials(api_key: String, base_url: String) -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(OpenAiConfig::new(
            api_key, base_url,
        )))
    }
}

impl MistralProvider {
    /// Create a Mistral provider from the `MISTRAL_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `MISTRAL_API_KEY` is not set.
    pub fn from_env() -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(MistralConfig::from_env()))
    }

    /// Create a Mistral provider with an explicit API key.
    pub fn with_api_key(api_key: String) -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(MistralConfig::new(api_key)))
    }
}
