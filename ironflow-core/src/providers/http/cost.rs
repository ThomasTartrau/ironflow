//! Token cost tables per provider and model.

use std::collections::HashMap;
#[cfg(any(
    feature = "provider-openai",
    feature = "provider-mistral",
    feature = "provider-gemini",
    feature = "provider-anthropic-api"
))]
use std::sync::LazyLock;

/// Cost per million tokens (input and output) for a model.
#[derive(Debug, Clone, Copy)]
pub struct CostEntry {
    /// USD per million input tokens.
    pub input_per_mtok: f64,
    /// USD per million output tokens.
    pub output_per_mtok: f64,
}

/// Lookup table mapping model identifiers to their token costs.
#[derive(Debug)]
pub struct CostTable {
    entries: HashMap<&'static str, CostEntry>,
}

impl CostTable {
    /// Compute cost in USD for the given token counts.
    pub fn compute(&self, model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
        self.entries.get(model).map(|e| {
            (input_tokens as f64 / 1_000_000.0) * e.input_per_mtok
                + (output_tokens as f64 / 1_000_000.0) * e.output_per_mtok
        })
    }
}

/// OpenAI model costs.
#[cfg(feature = "provider-openai")]
pub static OPENAI_COSTS: LazyLock<CostTable> = LazyLock::new(|| {
    use crate::providers::http::openai_compat::config::OpenAiModel;

    let entries = HashMap::from([
        (
            OpenAiModel::GPT_5_5,
            CostEntry {
                input_per_mtok: 5.00,
                output_per_mtok: 30.00,
            },
        ),
        (
            OpenAiModel::GPT_5_4,
            CostEntry {
                input_per_mtok: 2.50,
                output_per_mtok: 15.00,
            },
        ),
        (
            OpenAiModel::GPT_5_4_MINI,
            CostEntry {
                input_per_mtok: 0.75,
                output_per_mtok: 4.50,
            },
        ),
        (
            OpenAiModel::GPT_5_4_NANO,
            CostEntry {
                input_per_mtok: 0.20,
                output_per_mtok: 1.25,
            },
        ),
        (
            OpenAiModel::GPT_4_1,
            CostEntry {
                input_per_mtok: 2.00,
                output_per_mtok: 8.00,
            },
        ),
        (
            OpenAiModel::GPT_4_1_MINI,
            CostEntry {
                input_per_mtok: 0.40,
                output_per_mtok: 1.60,
            },
        ),
        (
            OpenAiModel::GPT_4_1_NANO,
            CostEntry {
                input_per_mtok: 0.10,
                output_per_mtok: 0.40,
            },
        ),
        (
            OpenAiModel::GPT_4O,
            CostEntry {
                input_per_mtok: 2.50,
                output_per_mtok: 10.00,
            },
        ),
        (
            OpenAiModel::GPT_4O_MINI,
            CostEntry {
                input_per_mtok: 0.15,
                output_per_mtok: 0.60,
            },
        ),
    ]);
    CostTable { entries }
});

/// Mistral model costs.
#[cfg(feature = "provider-mistral")]
pub static MISTRAL_COSTS: LazyLock<CostTable> = LazyLock::new(|| {
    use crate::providers::http::openai_compat::config::MistralModel;

    let entries = HashMap::from([
        (
            MistralModel::MEDIUM_3_5,
            CostEntry {
                input_per_mtok: 1.50,
                output_per_mtok: 7.50,
            },
        ),
        (
            MistralModel::LARGE,
            CostEntry {
                input_per_mtok: 0.50,
                output_per_mtok: 1.50,
            },
        ),
        (
            MistralModel::SMALL,
            CostEntry {
                input_per_mtok: 0.10,
                output_per_mtok: 0.30,
            },
        ),
        (
            MistralModel::MEDIUM,
            CostEntry {
                input_per_mtok: 1.00,
                output_per_mtok: 3.00,
            },
        ),
        (
            MistralModel::CODESTRAL,
            CostEntry {
                input_per_mtok: 0.30,
                output_per_mtok: 0.90,
            },
        ),
    ]);
    CostTable { entries }
});

/// Anthropic model costs.
#[cfg(feature = "provider-anthropic-api")]
pub static ANTHROPIC_COSTS: LazyLock<CostTable> = LazyLock::new(|| {
    use crate::providers::http::anthropic::adapter::AnthropicModel;

    let entries = HashMap::from([
        (
            AnthropicModel::OPUS_4_7,
            CostEntry {
                input_per_mtok: 5.00,
                output_per_mtok: 25.00,
            },
        ),
        (
            AnthropicModel::SONNET_4_6,
            CostEntry {
                input_per_mtok: 3.00,
                output_per_mtok: 15.00,
            },
        ),
        (
            AnthropicModel::HAIKU_4_5,
            CostEntry {
                input_per_mtok: 1.00,
                output_per_mtok: 5.00,
            },
        ),
        (
            AnthropicModel::OPUS_4_6,
            CostEntry {
                input_per_mtok: 5.00,
                output_per_mtok: 25.00,
            },
        ),
        (
            AnthropicModel::SONNET_4_5,
            CostEntry {
                input_per_mtok: 3.00,
                output_per_mtok: 15.00,
            },
        ),
    ]);
    CostTable { entries }
});

/// Google Gemini model costs.
#[cfg(feature = "provider-gemini")]
pub static GEMINI_COSTS: LazyLock<CostTable> = LazyLock::new(|| {
    use crate::providers::http::gemini::adapter::GeminiModel;

    let entries = HashMap::from([
        (
            GeminiModel::FLASH_3_5,
            CostEntry {
                input_per_mtok: 0.15,
                output_per_mtok: 0.60,
            },
        ),
        (
            GeminiModel::FLASH_LITE_3_1,
            CostEntry {
                input_per_mtok: 0.05,
                output_per_mtok: 0.20,
            },
        ),
        (
            GeminiModel::PRO_2_5,
            CostEntry {
                input_per_mtok: 1.25,
                output_per_mtok: 10.00,
            },
        ),
        (
            GeminiModel::FLASH_2_5,
            CostEntry {
                input_per_mtok: 0.15,
                output_per_mtok: 0.60,
            },
        ),
        (
            GeminiModel::FLASH_LITE_2_5,
            CostEntry {
                input_per_mtok: 0.05,
                output_per_mtok: 0.20,
            },
        ),
    ]);
    CostTable { entries }
});
