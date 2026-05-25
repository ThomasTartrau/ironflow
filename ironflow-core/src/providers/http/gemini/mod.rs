//! Google Gemini API provider.

pub mod adapter;

pub use adapter::GeminiModel;

use std::env;

use crate::providers::http::adapter::HttpAgentProvider;

use self::adapter::GeminiAdapter;

/// Google Gemini provider using the `generateContent` API.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
/// use ironflow_core::providers::http::GeminiProvider;
///
/// # async fn example() -> Result<(), OperationError> {
/// let provider = GeminiProvider::from_env();
/// let result = Agent::new()
///     .prompt("Say hello")
///     .model("gemini-2.5-flash")
///     .run(&provider)
///     .await?;
/// # Ok(())
/// # }
/// ```
pub type GeminiProvider = HttpAgentProvider<GeminiAdapter>;

impl GeminiProvider {
    /// Create from the `GEMINI_API_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics if `GEMINI_API_KEY` is not set.
    pub fn from_env() -> Self {
        let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY must be set");
        HttpAgentProvider::new(GeminiAdapter::new(api_key))
    }

    /// Create with an explicit API key.
    pub fn with_api_key(api_key: String) -> Self {
        HttpAgentProvider::new(GeminiAdapter::new(api_key))
    }
}
