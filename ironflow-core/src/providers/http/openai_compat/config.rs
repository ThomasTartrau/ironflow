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

/// Known NVIDIA NIM model identifiers.
///
/// NVIDIA NIM hosts 100+ models from multiple providers via a single
/// OpenAI-compatible API at `integrate.api.nvidia.com`. Model IDs verified
/// against the live `/v1/models` endpoint.
pub struct NvidiaModel;

impl NvidiaModel {
    // -- GLM (Z.ai / ex-Zhipu) --

    /// GLM 5.1 - Z.ai flagship for agentic workflows and reasoning.
    pub const GLM_5_1: &str = "z-ai/glm-5.1";

    // -- Kimi (Moonshot AI) --

    /// Kimi K2.6 - Moonshot AI latest model, 1T params MoE.
    pub const KIMI_K2_6: &str = "moonshotai/kimi-k2.6";

    // -- DeepSeek --

    /// DeepSeek V4 Pro - 1.6T params / 49B active, 1M context.
    pub const DEEPSEEK_V4_PRO: &str = "deepseek-ai/deepseek-v4-pro";
    /// DeepSeek V4 Flash - fast variant for coding and agents.
    pub const DEEPSEEK_V4_FLASH: &str = "deepseek-ai/deepseek-v4-flash";

    // -- Qwen (Alibaba) --

    /// Qwen 3.5 397B - Alibaba largest, 397B / 17B active (MoE).
    pub const QWEN3_5_397B: &str = "qwen/qwen3.5-397b-a17b";
    /// Qwen 3.5 122B - Alibaba flagship, 122B / 10B active (MoE).
    pub const QWEN3_5_122B: &str = "qwen/qwen3.5-122b-a10b";
    /// Qwen 3 Coder 480B - coding specialist, 480B / 35B active.
    pub const QWEN3_CODER: &str = "qwen/qwen3-coder-480b-a35b-instruct";
    /// Qwen 3 Next 80B Instruct - next-gen, 80B / 3B active.
    pub const QWEN3_NEXT: &str = "qwen/qwen3-next-80b-a3b-instruct";

    // -- Gemma (Google) --

    /// Gemma 4 31B IT - Google DeepMind latest.
    pub const GEMMA_4_31B: &str = "google/gemma-4-31b-it";
    /// Gemma 3n E4B IT - Google lightweight edge model.
    pub const GEMMA_3N_E4B: &str = "google/gemma-3n-e4b-it";
    /// Gemma 3 12B IT - Google mid-size.
    pub const GEMMA_3_12B: &str = "google/gemma-3-12b-it";

    // -- Llama (Meta) --

    /// Llama 4 Maverick 17B 128E - Meta MoE flagship.
    pub const LLAMA_4_MAVERICK: &str = "meta/llama-4-maverick-17b-128e-instruct";
    /// Llama 3.3 70B Instruct - Meta dense open-weight.
    pub const LLAMA_3_3_70B: &str = "meta/llama-3.3-70b-instruct";
    /// Llama 3.1 8B Instruct - Meta lightweight.
    pub const LLAMA_3_1_8B: &str = "meta/llama-3.1-8b-instruct";

    // -- Nemotron (NVIDIA) --

    /// Nemotron Ultra 253B - NVIDIA largest.
    pub const NEMOTRON_ULTRA_253B: &str = "nvidia/llama-3.1-nemotron-ultra-253b-v1";
    /// Nemotron Super 49B v1.5 - NVIDIA optimized Llama 3.3.
    pub const NEMOTRON_SUPER_49B: &str = "nvidia/llama-3.3-nemotron-super-49b-v1.5";
    /// Nemotron 3 Super 120B - NVIDIA large model, 120B / 12B active.
    pub const NEMOTRON_3_SUPER_120B: &str = "nvidia/nemotron-3-super-120b-a12b";
    /// Nemotron Nano 9B v2 - NVIDIA smallest and cheapest.
    pub const NEMOTRON_NANO_9B: &str = "nvidia/nvidia-nemotron-nano-9b-v2";

    // -- Others --

    /// MiniMax M2.7 - MiniMax flagship.
    pub const MINIMAX_M2_7: &str = "minimaxai/minimax-m2.7";
    /// GPT-OSS 120B - OpenAI open-source model on NVIDIA.
    pub const GPT_OSS_120B: &str = "openai/gpt-oss-120b";
    /// Mistral Large 3 675B - Mistral largest on NVIDIA.
    pub const MISTRAL_LARGE_3: &str = "mistralai/mistral-large-3-675b-instruct-2512";
    /// Mistral Medium 3.5 128B - Mistral coding/agentic on NVIDIA.
    pub const MISTRAL_MEDIUM_3_5: &str = "mistralai/mistral-medium-3.5-128b";
    /// Step 3.5 Flash - Stepfun fast model.
    pub const STEP_3_5_FLASH: &str = "stepfun-ai/step-3.5-flash";
    /// ByteDance Seed-OSS 36B.
    pub const SEED_OSS_36B: &str = "bytedance/seed-oss-36b-instruct";
}

/// NVIDIA NIM API configuration.
///
/// Connects to [build.nvidia.com](https://build.nvidia.com) which hosts 100+
/// models from multiple providers behind a single OpenAI-compatible endpoint.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::http::openai_compat::config::NvidiaConfig;
///
/// let config = NvidiaConfig::from_env();
/// ```
pub struct NvidiaConfig {
    api_key: String,
}

impl NvidiaConfig {
    /// Create from the `NVIDIA_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `NVIDIA_API_KEY` is not set.
    pub fn from_env() -> Self {
        Self {
            api_key: env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY must be set"),
        }
    }

    /// Create with an explicit API key.
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl OpenAiCompatConfig for NvidiaConfig {
    fn base_url(&self) -> &str {
        "https://integrate.api.nvidia.com/v1"
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn provider_name(&self) -> &'static str {
        "nvidia"
    }

    fn supports_json_schema(&self) -> bool {
        false
    }

    fn default_model(&self) -> &str {
        NvidiaModel::DEEPSEEK_V4_FLASH
    }

    fn small_model(&self) -> &str {
        NvidiaModel::NEMOTRON_NANO_9B
    }
}
