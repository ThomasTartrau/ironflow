//! HTTP-based LLM provider implementations.
//!
//! This module provides a shared adapter layer ([`HttpAgentAdapter`]) and generic
//! provider wrapper ([`HttpAgentProvider`]) that handle HTTP transport, SSE streaming,
//! timeout, and rate limiting. Concrete providers (OpenAI, Mistral, Gemini, Anthropic)
//! only implement request/response format differences.
//!
//! Enable providers via feature flags: `provider-openai`, `provider-mistral`,
//! `provider-gemini`, `provider-anthropic-api`.

pub mod adapter;
pub mod cost;
pub mod sse;

#[cfg(any(feature = "provider-openai", feature = "provider-mistral"))]
pub mod openai_compat;

#[cfg(feature = "provider-anthropic-api")]
pub mod anthropic;

#[cfg(feature = "provider-gemini")]
pub mod gemini;

pub use adapter::HttpAgentProvider;

#[cfg(feature = "provider-openai")]
pub use openai_compat::{OpenAiModel, OpenAiProvider};

#[cfg(feature = "provider-mistral")]
pub use openai_compat::{MistralModel, MistralProvider};

#[cfg(feature = "provider-anthropic-api")]
pub use anthropic::{AnthropicApiProvider, AnthropicModel};

#[cfg(feature = "provider-gemini")]
pub use gemini::{GeminiModel, GeminiProvider};
