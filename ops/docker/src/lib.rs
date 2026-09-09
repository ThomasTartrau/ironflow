//! Docker operations for Ironflow workflows, powered by [`bollard`].
//!
//! This crate provides Docker operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation wraps a [`bollard`] API call.
//!
//! # Architecture
//!
//! - [`DockerClient`] is the central handle, wrapping a [`bollard::Docker`] connection
//! - Each operation is a standalone struct implementing [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "docker"`
//! - Parameters are set at construction time via builder methods
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_docker::DockerClient;
//! use ironflow_ops_docker::containers::{ContainerCreate, ContainerStart, ContainerRemove};
//! use ironflow_ops_docker::system::SystemPing;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let client = DockerClient::connect_local()?;
//!
//! // Ping the daemon
//! let ping = SystemPing::new(&client);
//! ping.execute(&ctx).await?;
//!
//! // Create and start a container
//! let create = ContainerCreate::new(&client, "my-app", "alpine:latest")
//!     .cmd(vec!["sleep".into(), "3600".into()]);
//! let output = create.run(&ctx).await?;
//!
//! let start = ContainerStart::new(&client, &output.id);
//! start.run(&ctx).await?;
//!
//! // Cleanup
//! let remove = ContainerRemove::new(&client, &output.id).force();
//! remove.run(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Modules
//!
//! Operations are organized by Docker domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`containers`] | Create, Start, Stop, Restart, Kill, Remove, Inspect, List, Logs, Exec, Wait, Pause, Unpause, Rename, Top, Stats, Changes, Prune |
//! | [`images`] | List, Pull, Push, Inspect, Remove, Tag, History, Search, Prune |
//! | [`volumes`] | Create, Inspect, List, Remove, Prune |
//! | [`networks`] | Create, Inspect, List, Remove, Connect, Disconnect, Prune |
//! | [`system`] | Info, Version, Ping, Df |

mod client;
pub mod containers;
mod helpers;
pub mod images;
pub mod networks;
pub mod system;
pub mod volumes;

pub use bollard;
pub use client::DockerClient;
