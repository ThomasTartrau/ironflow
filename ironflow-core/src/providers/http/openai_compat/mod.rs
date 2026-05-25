//! OpenAI-compatible provider implementation (covers OpenAI, Mistral, and NVIDIA NIM).

pub mod adapter;
pub mod config;

#[cfg(feature = "provider-mistral")]
pub use config::MistralModel;
#[cfg(feature = "provider-nvidia")]
pub use config::NvidiaModel;
#[cfg(feature = "provider-openai")]
pub use config::OpenAiModel;

use crate::providers::http::adapter::HttpAgentProvider;

use self::adapter::OpenAiCompatAdapter;

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
#[cfg(feature = "provider-openai")]
pub type OpenAiProvider = HttpAgentProvider<OpenAiCompatAdapter<config::OpenAiConfig>>;

#[cfg(feature = "provider-openai")]
impl OpenAiProvider {
    /// Create an OpenAI provider from the `OPENAI_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `OPENAI_API_KEY` is not set.
    pub fn from_env() -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(config::OpenAiConfig::from_env()))
    }

    /// Create an OpenAI provider with explicit credentials.
    pub fn with_credentials(api_key: String, base_url: String) -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(config::OpenAiConfig::new(
            api_key, base_url,
        )))
    }
}

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
///     .model("mistral-medium-3.5")
///     .run(&provider)
///     .await?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "provider-mistral")]
pub type MistralProvider = HttpAgentProvider<OpenAiCompatAdapter<config::MistralConfig>>;

#[cfg(feature = "provider-mistral")]
impl MistralProvider {
    /// Create a Mistral provider from the `MISTRAL_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `MISTRAL_API_KEY` is not set.
    pub fn from_env() -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(config::MistralConfig::from_env()))
    }

    /// Create a Mistral provider with an explicit API key.
    pub fn with_api_key(api_key: String) -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(config::MistralConfig::new(
            api_key,
        )))
    }
}

/// NVIDIA NIM provider accessing 100+ models via a single OpenAI-compatible API.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
/// use ironflow_core::providers::http::NvidiaProvider;
/// use ironflow_core::providers::http::NvidiaModel;
///
/// # async fn example() -> Result<(), OperationError> {
/// let provider = NvidiaProvider::from_env();
/// let result = Agent::new()
///     .prompt("Say hello")
///     .model(NvidiaModel::DEEPSEEK_V4_FLASH)
///     .run(&provider)
///     .await?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "provider-nvidia")]
pub type NvidiaProvider = HttpAgentProvider<OpenAiCompatAdapter<config::NvidiaConfig>>;

#[cfg(feature = "provider-nvidia")]
impl NvidiaProvider {
    /// Create a NVIDIA NIM provider from the `NVIDIA_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `NVIDIA_API_KEY` is not set.
    pub fn from_env() -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(config::NvidiaConfig::from_env()))
    }

    /// Create a NVIDIA NIM provider with an explicit API key.
    pub fn with_api_key(api_key: String) -> Self {
        HttpAgentProvider::new(OpenAiCompatAdapter::new(config::NvidiaConfig::new(api_key)))
    }
}
