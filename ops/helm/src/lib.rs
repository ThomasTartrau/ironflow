//! Helm operations for Ironflow workflows, wrapping the Helm CLI.
//!
//! This crate provides Helm operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation spawns the `helm` binary via [`tokio::process::Command`] and
//! parses the output.
//!
//! # Architecture
//!
//! - [`HelmClient`] is the central handle, configuring the binary path,
//!   kubeconfig, and default namespace
//! - Each operation is a standalone struct implementing [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "helm"`
//! - Parameters are set at construction time via builder methods
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_helm::HelmClient;
//! use ironflow_ops_helm::release::Install;
//! use ironflow_ops_helm::util::Version;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let client = HelmClient::default();
//!
//! // Check Helm version
//! let version = Version::new(client.clone());
//! let output = version.execute(&ctx).await?;
//!
//! // Install a chart
//! let install = Install::new(client, "my-release", "bitnami/nginx")
//!     .wait(true)
//!     .create_namespace(true);
//! install.execute(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Tracked operations
//!
//! Every operation implements [`Operation`](ironflow_core::operation::Operation),
//! so it can be passed to `WorkflowContext::operation()` for step lifecycle
//! tracking (step record, status transitions, duration, output persistence).
//!
//! # Modules
//!
//! Operations are organized by Helm domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`release`] | Install, Upgrade, Uninstall, Rollback, List, Status, History, Get, Test |
//! | [`chart`] | Template, Lint, Package, Show, Pull, Push, DependencyUpdate/Build/List |
//! | [`repo`] | RepoAdd, RepoRemove, RepoUpdate, RepoList, RepoIndex, SearchRepo, SearchHub |
//! | [`registry`] | RegistryLogin, RegistryLogout |
//! | [`plugin`] | PluginInstall, PluginUninstall, PluginList, PluginUpdate |
//! | [`util`] | Version, EnvInfo, Verify |

pub mod chart;
mod client;
mod helpers;
pub mod plugin;
pub mod registry;
pub mod release;
pub mod repo;
pub mod util;

pub use client::HelmClient;
