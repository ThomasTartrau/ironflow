//! Kubernetes transports for Claude Code CLI.
//!
//! Two providers are available:
//!
//! * [`K8sEphemeralProvider`] - creates a new pod for each invocation, reads logs,
//!   then deletes the pod. Simple and isolated but has startup overhead.
//! * [`K8sPersistentProvider`] - reuses a long-running worker pod and executes
//!   commands via the Kubernetes exec API. Lower latency but shared state between
//!   invocations.
//!
//! Shared types ([`K8sResources`]) and helpers live in the [`common`] submodule.

pub mod common;
pub mod ephemeral;
pub mod persistent;

pub use common::{ImagePullPolicy, K8sClusterConfig, K8sResources};
pub use ephemeral::K8sEphemeralProvider;
pub use persistent::K8sPersistentProvider;
