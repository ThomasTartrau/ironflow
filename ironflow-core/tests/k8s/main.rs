//! Integration tests for the Kubernetes cleanup: the orphan reaper and the
//! release of a run's pods before it executes.
//!
//! The real `kube` client talks to a fake API server on a local TCP port
//! (see [`fake_api`]).

#![cfg(feature = "transport-k8s")]

mod fake_api;
mod reaper;
mod release_run;
