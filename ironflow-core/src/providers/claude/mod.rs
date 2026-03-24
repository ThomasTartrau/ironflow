//! Claude Code CLI provider implementations.
//!
//! This module contains all transport backends for invoking the `claude` CLI:
//!
//! * [`ClaudeCodeProvider`] — local process execution (always available).
//! * [`SshProvider`] — remote execution via SSH (requires `transport-ssh` feature).
//! * [`DockerProvider`] — execution inside a Docker container (requires `transport-docker` feature).
//! * [`K8sEphemeralProvider`] — one-shot Kubernetes pod per invocation (requires `transport-k8s` feature).
//! * [`K8sPersistentProvider`] — reuses a persistent Kubernetes worker pod (requires `transport-k8s` feature).
//!
//! All transports share command building and response parsing logic from the
//! [`common`] module.

pub mod common;
pub mod local;

#[cfg(feature = "transport-ssh")]
pub mod ssh;

#[cfg(feature = "transport-docker")]
pub mod docker;

#[cfg(feature = "transport-k8s")]
pub mod k8s;

pub use local::ClaudeCodeProvider;

#[cfg(feature = "transport-ssh")]
pub use ssh::SshProvider;

#[cfg(feature = "transport-docker")]
pub use docker::DockerProvider;

#[cfg(feature = "transport-k8s")]
pub use k8s::{
    ImagePullPolicy, K8sClusterConfig, K8sEphemeralProvider, K8sPersistentProvider, K8sResources,
};
