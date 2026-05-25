//! Configuration traits and structs for OpenAI-compatible providers.

use std::env;

/// Configuration for an OpenAI-compatible API endpoint.
///
/// Implemented by [`OpenAiConfig`] and [`MistralConfig`] to share the
/// same request/response logic with different credentials and base URLs.
pub trait OpenAiCompatConfig: Send + Sync + 'static {
    /// Base URL without trailing slash (e.g. `https://api.openai.com/v1`).
    fn base_url(&self) -> &str;

    /// API key for Bearer authentication.
    fn api_key(&self) -> &str;

    /// Provider name for logging and error messages.
    fn provider_name(&self) -> &'static str;

    /// Whether this provider supports strict JSON schema in `response_format`.
    fn supports_json_schema(&self) -> bool;

    /// Default model ID when the user passes a Claude alias.
    fn default_model(&self) -> &str;

    /// Model for lightweight tasks (maps from "haiku" alias).
    fn small_model(&self) -> &str;
}

/// Known OpenAI model identifiers.
pub struct OpenAiModel;

impl OpenAiModel {
    /// GPT-5.5 - current flagship model (1.05M context, 128k output).
    pub const GPT_5_5: &str = "gpt-5.5";
    /// GPT-5.4 - previous flagship (balanced cost/performance).
    pub const GPT_5_4: &str = "gpt-5.4";
    /// GPT-5.4 mini - fast and affordable.
    pub const GPT_5_4_MINI: &str = "gpt-5.4-mini";
    /// GPT-5.4 nano - smallest and cheapest.
    pub const GPT_5_4_NANO: &str = "gpt-5.4-nano";
    /// GPT-4.1 - legacy, excels at instruction following (1M context).
    pub const GPT_4_1: &str = "gpt-4.1";
    /// GPT-4.1 mini - legacy lightweight (1M context).
    pub const GPT_4_1_MINI: &str = "gpt-4.1-mini";
    /// GPT-4.1 nano - legacy fastest (1M context).
    pub const GPT_4_1_NANO: &str = "gpt-4.1-nano";
    /// GPT-4o - legacy multimodal (128k context).
    pub const GPT_4O: &str = "gpt-4o";
    /// GPT-4o mini - legacy lightweight multimodal (128k context).
    pub const GPT_4O_MINI: &str = "gpt-4o-mini";
}

/// Known Mistral model identifiers.
pub struct MistralModel;

impl MistralModel {
    /// Mistral Medium 3.5 - flagship agentic/coding model (256k context).
    pub const MEDIUM_3_5: &str = "mistral-medium-3.5";
    /// Mistral Large - open-weight multimodal, 41B active params (256k context).
    pub const LARGE: &str = "mistral-large-latest";
    /// Mistral Small - hybrid instruct/reasoning/coding (256k context).
    pub const SMALL: &str = "mistral-small-latest";
    /// Codestral - optimized for code generation and FIM (32k context).
    pub const CODESTRAL: &str = "codestral-latest";
    /// Mistral Medium (latest pointer).
    pub const MEDIUM: &str = "mistral-medium-latest";
}

/// OpenAI API configuration.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::http::openai_compat::config::OpenAiConfig;
///
/// let config = OpenAiConfig::from_env();
/// ```
pub struct OpenAiConfig {
    api_key: String,
    base_url: String,
}

impl OpenAiConfig {
    /// Create from the `OPENAI_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `OPENAI_API_KEY` is not set.
    pub fn from_env() -> Self {
        Self {
            api_key: env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set"),
            base_url: env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
        }
    }

    /// Create with explicit API key and base URL.
    pub fn new(api_key: String, base_url: String) -> Self {
        Self { api_key, base_url }
    }
}

impl OpenAiCompatConfig for OpenAiConfig {
    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn provider_name(&self) -> &'static str {
        "openai"
    }

    fn supports_json_schema(&self) -> bool {
        true
    }

    fn default_model(&self) -> &str {
        OpenAiModel::GPT_5_5
    }

    fn small_model(&self) -> &str {
        OpenAiModel::GPT_5_4_MINI
    }
}

/// Mistral API configuration.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::http::openai_compat::config::MistralConfig;
///
/// let config = MistralConfig::from_env();
/// ```
pub struct MistralConfig {
    api_key: String,
}

impl MistralConfig {
    /// Create from the `MISTRAL_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `MISTRAL_API_KEY` is not set.
    pub fn from_env() -> Self {
        Self {
            api_key: env::var("MISTRAL_API_KEY").expect("MISTRAL_API_KEY must be set"),
        }
    }

    /// Create with an explicit API key.
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl OpenAiCompatConfig for MistralConfig {
    fn base_url(&self) -> &str {
        "https://api.mistral.ai/v1"
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn provider_name(&self) -> &'static str {
        "mistral"
    }

    fn supports_json_schema(&self) -> bool {
        false
    }

    fn default_model(&self) -> &str {
        MistralModel::MEDIUM_3_5
    }

    fn small_model(&self) -> &str {
        MistralModel::SMALL
    }
}
