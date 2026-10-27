//! Integration tests for the Kubernetes cleanup: the orphan reaper, the
//! release of a run's pods before it executes, and the revocation of a
//! step's auth proxy tokens.
//!
//! The real `kube` client talks to a fake API server on a local TCP port
//! (see [`fake_api`]).

#![cfg(feature = "transport-k8s")]

mod fake_api;
mod owner;
mod proxy_tokens;
mod reaper;
mod release_owned;
mod release_run;
