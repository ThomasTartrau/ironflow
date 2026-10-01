//! Kubernetes transports for Claude Code CLI.
//!
//! Two providers are available:
//!
//! * [`K8sEphemeralProvider`] - creates a new pod for each invocation, reads logs,
//!   then deletes the pod. Simple and isolated but has startup overhead.
//!   [`K8sEphemeralProvider::sandboxed`] adds a hardened pod (non-root,
//!   read-only root filesystem, secrets from Kubernetes Secrets, managed
//!   settings presets, egress profile label, Claude profile ConfigMaps in
//!   [`profile`]), cleanup of a previous attempt's pods on retry and of a
//!   run's pods before it executes again, and an orphan reaper ([`reap_orphans`]).
//!   [`K8sEphemeralProvider::auth_proxy`] routes Claude traffic through an
//!   `ironflow-auth-proxy`: the pod gets an opaque per-step token instead of
//!   the Claude credential.
//! * [`K8sPersistentProvider`] - reuses a long-running worker pod and executes
//!   commands via the Kubernetes exec API. Lower latency but shared state between
//!   invocations.
//!
//! Shared types ([`K8sResources`], [`PodHardening`], [`SandboxSettings`]) and
//! helpers live in the [`common`] submodule; the orphan reaping decisions
//! ([`reap_reason`], [`configmap_expired`]) in [`reaper`].

mod cleanup;
pub mod common;
pub mod ephemeral;
pub mod persistent;
pub mod profile;
pub mod reaper;
pub mod toleration;

pub use common::{ImagePullPolicy, K8sClusterConfig, K8sResources, PodHardening, SandboxSettings};
pub use ephemeral::K8sEphemeralProvider;
pub use persistent::K8sPersistentProvider;
pub use reaper::{
    ReapReason, ReapReport, configmap_expired, job_reap_reason, reap_orphans, reap_reason,
};
pub use toleration::{K8sToleration, TolerationEffect, TolerationOperator};
