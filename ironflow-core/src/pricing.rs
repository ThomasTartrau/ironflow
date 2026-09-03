//! Unified LLM cost attribution per step and workflow.
//!
//! Provides a provider-agnostic pricing interface ([`PricingSource`]) with a
//! built-in static implementation ([`StaticPricing`]) that covers all supported
//! model families. Cost is computed into a [`CostBreakdown`] with prompt and
//! completion components, rounded to 6 decimal places.
//!
//! The [`spawn_log`] helper emits cost telemetry in a fire-and-forget task so
//! tracking never blocks step execution.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::pricing::{CostBreakdown, PricingSource, StaticPricing};
//!
//! let pricing = StaticPricing::new();
//! let breakdown = CostBreakdown::compute(&pricing, "claude-sonnet-4-6", 1000, 500);
//! println!("total: ${:.6}", breakdown.total_usd);
//! ```

use tracing::{info, warn};

/// A source of per-model token pricing.
///
/// Implementations return the cost per million tokens (input, output) for a
/// given model identifier. A `None` return means the model is not in the
/// catalog; callers should apply a fallback.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::pricing::{PricingSource, StaticPricing};
///
/// let pricing = StaticPricing::new();
/// if let Some((input, output)) = pricing.price_per_1m("claude-sonnet-4-6") {
///     println!("input: ${input}/Mtok, output: ${output}/Mtok");
/// }
/// ```
pub trait PricingSource: Send + Sync {
    /// Return `(input_per_1m_usd, output_per_1m_usd)` for the given model.
    ///
    /// Returns `None` when the model is not in the catalog.
    fn price_per_1m(&self, model: &str) -> Option<(f64, f64)>;
}

/// Computed cost for a single LLM call, split into prompt and completion.
///
/// All values are in USD, rounded to 6 decimal places.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::pricing::{CostBreakdown, StaticPricing};
///
/// let pricing = StaticPricing::new();
/// let bd = CostBreakdown::compute(&pricing, "claude-sonnet-4-6", 10_000, 2_000);
/// assert!(bd.total_usd > 0.0);
/// assert!((bd.total_usd - bd.prompt_usd - bd.completion_usd).abs() < 1e-9);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostBreakdown {
    /// Cost of prompt (input) tokens in USD.
    pub prompt_usd: f64,
    /// Cost of completion (output) tokens in USD.
    pub completion_usd: f64,
    /// Total cost in USD (`prompt_usd + completion_usd`).
    pub total_usd: f64,
}

/// Round to 6 decimal places.
fn round6(v: f64) -> f64 {
    (v * 1_000_000.0).round() / 1_000_000.0
}

/// Conservative fallback price per million tokens (input, output).
/// Uses the Claude Sonnet rate ($3/$15), shared across Sonnet 4.5/4.6/5.
const SONNET_FALLBACK: (f64, f64) = (3.0, 15.0);

impl CostBreakdown {
    /// Compute the cost for an LLM call using the given pricing source.
    ///
    /// When the model is unknown the fallback Sonnet price is used and a
    /// warning is logged. The cost is never zero for a non-zero token count.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::pricing::{CostBreakdown, StaticPricing};
    ///
    /// let pricing = StaticPricing::new();
    /// let bd = CostBreakdown::compute(&pricing, "claude-opus-5", 5000, 1000);
    /// println!("${:.6}", bd.total_usd);
    /// ```
    pub fn compute(
        source: &dyn PricingSource,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Self {
        let (inp_rate, out_rate) = source.price_per_1m(model).unwrap_or_else(|| {
            warn!(
                model,
                fallback_input = SONNET_FALLBACK.0,
                fallback_output = SONNET_FALLBACK.1,
                "unknown model, falling back to Claude Sonnet pricing"
            );
            SONNET_FALLBACK
        });

        let prompt_usd = round6(input_tokens as f64 / 1_000_000.0 * inp_rate);
        let completion_usd = round6(output_tokens as f64 / 1_000_000.0 * out_rate);
        let total_usd = round6(prompt_usd + completion_usd);

        Self {
            prompt_usd,
            completion_usd,
            total_usd,
        }
    }
}

/// Static, hardcoded pricing table for all supported model families.
///
/// Resolution is by substring: the most specific (longest) matching entry
/// wins. For example, `"claude-sonnet-4-6[1m]"` matches the `"claude-sonnet-4-6"`
/// entry because the full model string contains that substring.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::pricing::{PricingSource, StaticPricing};
///
/// let p = StaticPricing::new();
/// // Exact match
/// assert!(p.price_per_1m("claude-opus-5").is_some());
/// // Substring match (model with [1m] suffix)
/// assert!(p.price_per_1m("claude-opus-5[1m]").is_some());
/// ```
pub struct StaticPricing {
    entries: Vec<(&'static str, f64, f64)>,
}

impl StaticPricing {
    /// Create a new static pricing table with all known models.
    #[must_use]
    pub fn new() -> Self {
        let mut entries = vec![
            // ── Anthropic ─────────────────────────────────────
            ("claude-fable-5", 10.0, 50.0),
            ("claude-fable-5-1", 10.0, 50.0),
            ("claude-mythos-5", 10.0, 50.0),
            ("claude-opus-5", 5.0, 25.0),
            ("claude-sonnet-5", 3.0, 15.0),
            ("claude-opus-4-8", 5.0, 25.0),
            ("claude-opus-4-7", 5.0, 25.0),
            ("claude-sonnet-4-6", 3.0, 15.0),
            ("claude-opus-4-6", 5.0, 25.0),
            ("claude-sonnet-4-5", 3.0, 15.0),
            ("claude-haiku-4-5", 1.0, 5.0),
            // Aliases used by ClaudeCodeProvider
            ("sonnet", 3.0, 15.0),
            ("opus", 5.0, 25.0),
            ("haiku", 1.0, 5.0),
            // ── OpenAI ────────────────────────────────────────
            ("gpt-5.5", 5.0, 30.0),
            ("gpt-5.4-mini", 0.75, 4.5),
            ("gpt-5.4-nano", 0.20, 1.25),
            ("gpt-5.4", 2.5, 15.0),
            ("gpt-4.1-mini", 0.40, 1.60),
            ("gpt-4.1-nano", 0.10, 0.40),
            ("gpt-4.1", 2.0, 8.0),
            ("gpt-4o-mini", 0.15, 0.60),
            ("gpt-4o", 2.5, 10.0),
            // ── Mistral ───────────────────────────────────────
            ("mistral-medium-3.5", 1.5, 7.5),
            ("mistral-large", 0.50, 1.50),
            ("mistral-small", 0.10, 0.30),
            ("mistral-medium", 1.0, 3.0),
            ("codestral", 0.30, 0.90),
            // ── Google Gemini ─────────────────────────────────
            ("gemini-3.5-flash", 0.15, 0.60),
            ("gemini-3.1-flash-lite", 0.05, 0.20),
            ("gemini-2.5-pro", 1.25, 10.0),
            ("gemini-2.5-flash", 0.15, 0.60),
            ("gemini-2.5-flash-lite", 0.05, 0.20),
            // ── NVIDIA NIM ────────────────────────────────────
            ("nemotron-nano-9b", 0.04, 0.16),
            ("nemotron-super-49b", 0.10, 0.40),
            ("nemotron-ultra-253b", 0.90, 0.90),
        ];
        // Sort by key length descending for longest-match-first resolution.
        entries.sort_by_key(|e| std::cmp::Reverse(e.0.len()));
        Self { entries }
    }
}

impl Default for StaticPricing {
    fn default() -> Self {
        Self::new()
    }
}

impl PricingSource for StaticPricing {
    fn price_per_1m(&self, model: &str) -> Option<(f64, f64)> {
        self.entries
            .iter()
            .find(|(key, _, _)| model.contains(key))
            .map(|(_, inp, out)| (*inp, *out))
    }
}

/// Emit a cost log line in a fire-and-forget tokio task.
///
/// The spawned task logs the cost breakdown via [`tracing::info!`] and never
/// blocks the calling step. Errors in the task are silently absorbed.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::pricing::{CostBreakdown, StaticPricing, spawn_log};
///
/// let pricing = StaticPricing::new();
/// let bd = CostBreakdown::compute(&pricing, "claude-opus-5", 5000, 1000);
/// spawn_log("my-step", "claude-opus-5", bd);
/// ```
pub fn spawn_log(step_name: &str, model: &str, breakdown: CostBreakdown) {
    let step = step_name.to_string();
    let model = model.to_string();
    tokio::spawn(async move {
        info!(
            step = %step,
            model = %model,
            prompt_usd = breakdown.prompt_usd,
            completion_usd = breakdown.completion_usd,
            total_usd = breakdown.total_usd,
            "agent step cost"
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_returns_correct_price() {
        let pricing = StaticPricing::new();
        let price = pricing.price_per_1m("claude-opus-5");
        assert_eq!(price, Some((5.0, 25.0)));
    }

    #[test]
    fn substring_resolution_most_specific_wins() {
        let pricing = StaticPricing::new();
        // "claude-sonnet-4-6[1m]" contains "claude-sonnet-4-6" (17 chars)
        // and also "sonnet" (6 chars). The longer key wins.
        let price = pricing.price_per_1m("claude-sonnet-4-6[1m]");
        assert_eq!(price, Some((3.0, 15.0)));

        // "gpt-4.1-mini" contains "gpt-4.1-mini" (12 chars) and "gpt-4.1" (7 chars).
        // The longer key wins.
        let price = pricing.price_per_1m("gpt-4.1-mini");
        assert_eq!(price, Some((0.40, 1.60)));
    }

    #[test]
    fn unknown_model_falls_back_to_sonnet() {
        let pricing = StaticPricing::new();
        // Unknown model -> None from price_per_1m
        assert!(pricing.price_per_1m("totally-unknown-model-xyz").is_none());

        // CostBreakdown::compute applies the fallback
        let bd =
            CostBreakdown::compute(&pricing, "totally-unknown-model-xyz", 1_000_000, 1_000_000);
        // Sonnet fallback: $3/Mtok input, $15/Mtok output
        assert_eq!(bd.prompt_usd, 3.0);
        assert_eq!(bd.completion_usd, 15.0);
        assert_eq!(bd.total_usd, 18.0);
    }

    #[test]
    fn cost_breakdown_rounds_to_six_decimals() {
        let pricing = StaticPricing::new();
        // 7 input tokens at $5/Mtok = 0.000035 (exact)
        // 3 output tokens at $25/Mtok = 0.000075 (exact)
        let bd = CostBreakdown::compute(&pricing, "claude-opus-5", 7, 3);
        assert_eq!(bd.prompt_usd, 0.000035);
        assert_eq!(bd.completion_usd, 0.000075);
        assert_eq!(bd.total_usd, 0.00011);

        // 1 input token at $3/Mtok = 0.000003 (exact)
        // 1 output token at $15/Mtok = 0.000015 (exact)
        let bd = CostBreakdown::compute(&pricing, "claude-sonnet-4-6", 1, 1);
        assert_eq!(bd.prompt_usd, 0.000003);
        assert_eq!(bd.completion_usd, 0.000015);
        assert_eq!(bd.total_usd, 0.000018);

        // Test rounding: 3 input tokens at $1.25/Mtok = 0.00000375 -> 0.000004
        let bd = CostBreakdown::compute(&pricing, "gemini-2.5-pro", 3, 0);
        assert_eq!(bd.prompt_usd, 0.000004);
    }

    #[test]
    fn zero_tokens_returns_zero_cost() {
        let pricing = StaticPricing::new();
        let bd = CostBreakdown::compute(&pricing, "claude-opus-5", 0, 0);
        assert_eq!(bd.prompt_usd, 0.0);
        assert_eq!(bd.completion_usd, 0.0);
        assert_eq!(bd.total_usd, 0.0);
    }

    #[tokio::test]
    async fn spawn_log_does_not_block() {
        use std::time::Duration;

        tokio::time::timeout(Duration::from_secs(5), async {
            let pricing = StaticPricing::new();
            let bd = CostBreakdown::compute(&pricing, "claude-opus-5", 1000, 500);
            spawn_log("test-step", "claude-opus-5", bd);
            tokio::task::yield_now().await;
        })
        .await
        .expect("spawn_log timed out");
    }

    #[test]
    fn all_known_models_have_prices() {
        let pricing = StaticPricing::new();
        let models = [
            "claude-fable-5",
            "claude-mythos-5",
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-sonnet-4-6",
            "claude-opus-4-6",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
            "sonnet",
            "opus",
            "haiku",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-4.1",
            "gpt-4o",
            "gpt-4o-mini",
            "mistral-large",
            "mistral-small",
            "codestral",
            "gemini-2.5-pro",
            "gemini-2.5-flash",
            "nemotron-nano-9b",
            "nemotron-super-49b",
            "nemotron-ultra-253b",
        ];
        for model in models {
            assert!(
                pricing.price_per_1m(model).is_some(),
                "missing price for {model}"
            );
        }
    }

    #[test]
    fn entries_sorted_by_length_descending() {
        let pricing = StaticPricing::new();
        for window in pricing.entries.windows(2) {
            assert!(
                window[0].0.len() >= window[1].0.len(),
                "entries not sorted: {:?} before {:?}",
                window[0].0,
                window[1].0
            );
        }
    }

    #[test]
    fn default_matches_new() {
        let a = StaticPricing::new();
        let b = StaticPricing::default();
        assert_eq!(a.entries.len(), b.entries.len());
        for (ea, eb) in a.entries.iter().zip(b.entries.iter()) {
            assert_eq!(ea.0, eb.0);
            assert_eq!(ea.1, eb.1);
            assert_eq!(ea.2, eb.2);
        }
    }

    #[test]
    fn cost_breakdown_with_large_token_counts() {
        let pricing = StaticPricing::new();
        // 1M input tokens at $5/Mtok = $5.0
        // 500K output tokens at $25/Mtok = $12.5
        let bd = CostBreakdown::compute(&pricing, "claude-opus-5", 1_000_000, 500_000);
        assert_eq!(bd.prompt_usd, 5.0);
        assert_eq!(bd.completion_usd, 12.5);
        assert_eq!(bd.total_usd, 17.5);
    }

    #[test]
    fn sonnet_fallback_never_zero() {
        let pricing = StaticPricing::new();
        // Even 1 token should produce a non-zero cost via fallback
        let bd = CostBreakdown::compute(&pricing, "unknown-model", 1, 0);
        assert!(bd.total_usd > 0.0);
    }
}
