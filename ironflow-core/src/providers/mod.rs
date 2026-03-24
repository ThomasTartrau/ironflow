//! Built-in [`AgentProvider`](crate::provider::AgentProvider) implementations.
//!
//! * [`claude::ClaudeCodeProvider`] — local execution of the `claude` CLI.
//! * [`claude::SshProvider`] — remote execution via SSH (requires `transport-ssh` feature).
//! * [`claude::DockerProvider`] — execution inside a Docker container (requires `transport-docker` feature).
//! * [`claude::K8sEphemeralProvider`] — one-shot Kubernetes pod (requires `transport-k8s` feature).
//! * [`claude::K8sPersistentProvider`] — persistent Kubernetes worker pod (requires `transport-k8s` feature).
//! * [`record_replay::RecordReplayProvider`] — test-friendly wrapper that
//!   records and replays agent responses from JSON fixtures.

pub mod claude;
pub mod record_replay;
