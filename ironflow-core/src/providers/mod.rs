//! Built-in [`AgentProvider`](crate::provider::AgentProvider) implementations.
//!
//! * [`claude::ClaudeCodeProvider`] - local execution of the `claude` CLI.
//! * `claude::SshProvider` - remote execution via SSH (requires `transport-ssh` feature).
//! * `claude::DockerProvider` - Docker container execution (requires `transport-docker` feature).
//! * `claude::K8sEphemeralProvider` - one-shot Kubernetes pod (requires `transport-k8s` feature).
//! * `claude::K8sPersistentProvider` - persistent Kubernetes worker pod (requires `transport-k8s` feature).
//! * [`record_replay::RecordReplayProvider`] - test-friendly wrapper that
//!   records and replays agent responses from JSON fixtures.
//! * `http::OpenAiProvider` - OpenAI Chat Completions API (requires `provider-openai` feature).
//! * `http::MistralProvider` - Mistral Chat Completions API (requires `provider-mistral` feature).
//! * `http::GeminiProvider` - Google Gemini generateContent API (requires `provider-gemini` feature).
//! * `http::AnthropicApiProvider` - Anthropic Messages API (requires `provider-anthropic-api` feature).

pub mod claude;
pub mod http;
pub mod record_replay;
