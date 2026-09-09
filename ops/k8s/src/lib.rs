//! Kubernetes integration for Ironflow workflows.
//!
//! This crate provides a thin integration layer between the
//! [`kube`](https://crates.io/crates/kube) crate and Ironflow's workflow
//! engine. It re-exports the full `kube` and `k8s_openapi` APIs so that
//! workflow handlers get typed access to every Kubernetes resource and verb
//! without managing authentication or HTTP clients manually.
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_k8s::KubeClient;
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let kube = KubeClient::from_context(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Typed resource access
//!
//! Use the re-exported [`kube::Api`] for direct API calls:
//!
//! ```no_run
//! use ironflow_ops_k8s::KubeClient;
//! use k8s_openapi::api::core::v1::Pod;
//! use kube::Config;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let config = Config::infer().await.expect("kubeconfig");
//! let kube = KubeClient::from_config(config).await?;
//! let pods = kube.namespaced::<Pod>("default");
//! // pods.list(&Default::default()).await?
//! # Ok(())
//! # }
//! ```
//!
//! # Tracked operations
//!
//! Wrap any verb in [`KubeOp`] to execute it as a tracked workflow step
//! via `WorkflowContext::operation()`:
//!
//! ```no_run
//! use ironflow_ops_k8s::{KubeClient, verb};
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use k8s_openapi::api::core::v1::Pod;
//! use kube::Config;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let config = Config::infer().await.expect("kubeconfig");
//! let kube = KubeClient::from_config(config).await?;
//! let pods = kube.namespaced::<Pod>("default");
//! let op = kube.op(pods, verb::List::default());
//! // op implements Operation -- pass it to ctx.operation("list-pods", &op)
//! # Ok(())
//! # }
//! ```
//!
//! # Streaming operations
//!
//! Watch, exec, attach, and port-forward are streaming operations that do not
//! fit the [`Operation`](ironflow_core::operation::Operation) trait (which
//! returns a single [`Value`](serde_json::Value)). Access them directly via
//! [`KubeClient::client`] and the [`kube::Api`] methods.

mod client;
pub(crate) mod error;
pub mod helpers;
pub mod operation;
pub mod verb;

pub use client::KubeClient;
pub use k8s_openapi;
pub use kube;
pub use operation::KubeOp;
